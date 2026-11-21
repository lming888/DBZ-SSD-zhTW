//! The window: the SDL3 window with its wgpu surface, the cursors, the clipboard and the text input
//! of the interface. What is drawn is [`Canvas`]'s, shared with the UI snapshots.

use crate::canvas::{self, Canvas, GameImage, GameView};
use crate::egui_input::{Geometry, InputAdapter};
use sdl3::clipboard::ClipboardUtil;
use sdl3::event::Event;
use sdl3::keyboard::TextInputUtil;
use sdl3::mouse::{Cursor, SystemCursor};
use sdl3::video::Window;
use std::time::Instant;

pub(crate) fn should_render(width: u32, height: u32) -> bool {
    width > 0 && height > 0
}

/// The surface format of the window. egui draws in gamma space, so a format that is not sRGB-aware
/// gives exact colours: `Bgra8Unorm` and `Rgba8Unorm` first, then any other such format, then the
/// first one the surface offers.
pub(crate) fn pick_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm]
        .into_iter()
        .find(|format| formats.contains(format))
        .or_else(|| formats.iter().copied().find(|format| !format.is_srgb()))
        .or_else(|| formats.first().copied())
}

/// The presentation mode of the Video page's V-sync: waits for the screen's refresh when on, else shows frames as soon as
/// they are ready (the main loop paces them itself).
pub(crate) fn present_mode(vsync: bool) -> wgpu::PresentMode {
    if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync }
}

/// A surface configuration that is due but not made yet. Configuring a surface while one of its
/// textures is alive panics in wgpu, so a `Suboptimal` texture is drawn and presented as it is, and
/// the surface is configured again at the start of the next present.
#[derive(Debug, Default, PartialEq, Eq)]
struct Reconfigure {
    due: bool,
}

impl Reconfigure {
    fn request(&mut self) {
        self.due = true;
    }

    /// Whether a configuration is due; taking it clears the request.
    fn take(&mut self) -> bool {
        std::mem::take(&mut self.due)
    }
}

/// The three mouse cursors the interface uses, created when first needed.
#[derive(Default)]
struct Cursors {
    shown: Option<CursorKind>,
    cursors: [Option<Cursor>; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorKind {
    Arrow,
    Hand,
    IBeam,
}

impl CursorKind {
    fn of(icon: egui::CursorIcon) -> Self {
        match icon {
            egui::CursorIcon::PointingHand | egui::CursorIcon::Grab => Self::Hand,
            egui::CursorIcon::Text | egui::CursorIcon::VerticalText => Self::IBeam,
            _ => Self::Arrow,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn system(self) -> SystemCursor {
        match self {
            Self::Arrow => SystemCursor::Arrow,
            Self::Hand => SystemCursor::Hand,
            Self::IBeam => SystemCursor::IBeam,
        }
    }
}

impl Cursors {
    fn show(&mut self, icon: egui::CursorIcon) {
        let kind = CursorKind::of(icon);
        if self.shown == Some(kind) {
            return;
        }
        let slot = &mut self.cursors[kind.index()];
        if slot.is_none() {
            *slot = Cursor::from_system(kind.system()).ok();
        }
        if let Some(cursor) = slot {
            cursor.set();
            self.shown = Some(kind);
        }
    }
}

/// What a present came to: Android counts them while a start has not shown its first picture
/// (`android_report::present_outcome`, a black screen told apart from a crash); nothing elsewhere.
#[derive(Debug, Clone, Copy)]
enum Outcome {
    Shown,
    Skipped,
    Outdated,
    Lost,
    Timeout,
    Occluded,
}

/// Passes `outcome` on (Android); whether the surface is to be made again (a black screen, once a start).
fn note(outcome: Outcome) -> bool {
    #[cfg(target_os = "android")]
    {
        use crate::android_report::PresentOutcome as P;
        crate::android_report::present_outcome(match outcome {
            Outcome::Shown => P::Shown,
            Outcome::Skipped => P::Skipped,
            Outcome::Outdated => P::Outdated,
            Outcome::Lost => P::Lost,
            Outcome::Timeout => P::Timeout,
            Outcome::Occluded => P::Occluded,
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = outcome;
        false
    }
}

pub struct Video {
    canvas: Canvas,
    // SAFETY: the surface must not outlive the window it was created from, so the window is
    // declared after it and dropped after it.
    surface_config: wgpu::SurfaceConfiguration,
    /// Gone while the app is in the background on Android (its native window is lost); see [`Video::suspend`].
    surface: Option<Attached>,
    instance: wgpu::Instance,
    cursors: Cursors,
    clipboard: ClipboardUtil,
    text_input: TextInputUtil,
    text_input_on: bool,
    minimized: bool,
    fullscreen: bool,
    reconfigure: Reconfigure,
    started: Instant,
    window: Window,
}

#[cfg(target_os = "android")]
#[link(name = "android")]
unsafe extern "C" {
    fn ANativeWindow_acquire(window: *mut std::ffi::c_void);
    fn ANativeWindow_release(window: *mut std::ffi::c_void);
}

/// Tries `attempt` until it gives something (`Ok(Some)`) or fails (`Err`), waiting with `pause` while it gives nothing
/// for want of the app's native surface (`Ok(None)`: the app is in the background; SDL has one again when it is back).
/// The wait has no end: time in the background (or frozen there) says nothing of the graphics, and the process is
/// usually reclaimed first; a start that waits in the foreground is the stall rule's (`launch_guard::stalled`, the
/// activity counts that time). `waited` is how long it has waited, for the log.
#[cfg(any(target_os = "android", test))]
fn wait_for_native<T>(
    what: &str,
    mut attempt: impl FnMut() -> anyhow::Result<Option<T>>,
    mut pause: impl FnMut(),
    waited: impl Fn() -> std::time::Duration,
) -> anyhow::Result<T> {
    let mut waiting = false;
    loop {
        if let Some(done) = attempt()? {
            if waiting {
                log::info!(
                    "{what}: the native surface is back after {:.1} s",
                    waited().as_secs_f64()
                );
            }
            return Ok(done);
        }
        if !waiting {
            waiting = true;
            log::info!("{what}: no native surface yet (in the background): waiting for it");
        }
        pause();
    }
}

/// Test builds only: `SSD_TEST_FAIL=slow-adapter` or `slow-device` holds the game's thread 3 s before the graphics
/// adapter is asked for, or after the device is made (before the surface's formats are), the first time only: a Home
/// then loses the native window in the middle of opening the graphics.
#[cfg(all(target_os = "android", debug_assertions))]
fn test_pause(point: &str) {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if std::env::var("SSD_TEST_FAIL").as_deref() == Ok(point)
        && !DONE.swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        log::warn!("SSD_TEST_FAIL={point}: the graphics wait 3 s");
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
}

/// Android: a reference of the window's native surface (SDL's `ANativeWindow`), held while the wgpu surface made from
/// it lives. When the app goes to the background the system destroys that surface and SDL lets its own reference go
/// (and sets the window's pointer to null): without this one, a wgpu surface made or configured a moment before used
/// a freed window, and the driver crashed (seen on the emulator: Home while the game opens its window, SIGABRT in
/// `eglCreateWindowSurface`, `pthread_mutex_lock called on a destroyed mutex`). Held, the window is only abandoned:
/// the driver's calls on it fail, and the surface is made again in the foreground. Elsewhere nothing.
struct Native {
    #[cfg(target_os = "android")]
    window: std::ptr::NonNull<std::ffi::c_void>,
}

/// Android: SDL's `ANativeWindow` of `window` now, null while it has none (the app in the background): SDL sets it to
/// null in the activity's `surfaceDestroyed`, before the system abandons that surface, and to the new one when the app
/// is back.
#[cfg(target_os = "android")]
fn native_pointer(window: &Window) -> *mut std::ffi::c_void {
    use sdl3::sys::properties::SDL_GetPointerProperty;
    use sdl3::sys::video::{SDL_GetWindowProperties, SDL_PROP_WINDOW_ANDROID_WINDOW_POINTER};
    // SAFETY: the properties of a live SDL window; the pointer is SDL's `ANativeWindow`, null without a surface.
    unsafe {
        SDL_GetPointerProperty(
            SDL_GetWindowProperties(window.raw()),
            SDL_PROP_WINDOW_ANDROID_WINDOW_POINTER,
            std::ptr::null_mut(),
        )
    }
}

impl Native {
    /// The native surface of `window` now; an error while it has none (the app in the background).
    #[cfg(target_os = "android")]
    fn of(window: &Window) -> anyhow::Result<Self> {
        let window = std::ptr::NonNull::new(native_pointer(window)).ok_or_else(|| {
            anyhow::anyhow!("the window has no native surface now (in the background)")
        })?;
        // SAFETY: a live `ANativeWindow` of SDL's; the reference taken here is given back in `drop`.
        unsafe { ANativeWindow_acquire(window.as_ptr()) };
        Ok(Self { window })
    }

    #[cfg(not(target_os = "android"))]
    #[allow(clippy::unnecessary_wraps)]
    fn of(_window: &Window) -> anyhow::Result<Self> {
        Ok(Self {})
    }

    /// Whether this is still `window`'s native surface: not when the app went to the background since it was taken
    /// (SDL let it go, and has none or a new one). A graphics call that failed on a surface no longer current failed
    /// for the background, not for the graphics. Elsewhere always.
    #[cfg(target_os = "android")]
    fn is_current(&self, window: &Window) -> bool {
        native_pointer(window) == self.window.as_ptr()
    }

    #[cfg(not(target_os = "android"))]
    #[allow(clippy::unused_self)]
    fn is_current(&self, _window: &Window) -> bool {
        true
    }

    /// As [`Native::of`], waiting while the window has no native surface (the app in the background as the window was
    /// made) for as long as it takes ([`wait_for_native`]): SDL sets it again when the app is back.
    #[cfg(target_os = "android")]
    fn of_waiting(window: &Window) -> anyhow::Result<Self> {
        let started = Instant::now();
        wait_for_native(
            "the wgpu surface",
            || {
                // Test builds only: `SSD_TEST_FAIL=native-late`, no native surface for the first 2 s.
                #[cfg(debug_assertions)]
                if std::env::var("SSD_TEST_FAIL").as_deref() == Ok("native-late")
                    && started.elapsed() < std::time::Duration::from_secs(2)
                {
                    return Ok(None);
                }
                Ok(Self::of(window).ok())
            },
            || std::thread::sleep(std::time::Duration::from_millis(50)),
            || started.elapsed(),
        )
    }

    #[cfg(not(target_os = "android"))]
    fn of_waiting(window: &Window) -> anyhow::Result<Self> {
        Self::of(window)
    }

    /// The wgpu target of the surface: the held native window.
    #[cfg(target_os = "android")]
    fn target(&self) -> wgpu::SurfaceTargetUnsafe {
        use wgpu::rwh::{
            AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle,
        };
        wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Android(AndroidDisplayHandle::new())),
            raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(
                self.window,
            )),
        }
    }
}

#[cfg(target_os = "android")]
impl Drop for Native {
    fn drop(&mut self) {
        // SAFETY: the reference `of` took.
        unsafe { ANativeWindow_release(self.window.as_ptr()) };
    }
}

/// The window of `builder`. On Android SDL refuses to make it while the app has no native surface (sent to the
/// background as the game opens its window: "Could not fetch native window"): that is no failure of the graphics, so
/// it is waited for until the app is back, as [`Native::of_waiting`] does ([`wait_for_native`]).
fn build_window(builder: &sdl3::video::WindowBuilder) -> anyhow::Result<Window> {
    #[cfg(target_os = "android")]
    {
        let started = Instant::now();
        wait_for_native(
            "the window",
            || match builder.build() {
                Ok(window) => Ok(Some(window)),
                Err(error) if error.to_string().contains("native window") => Ok(None),
                Err(error) => Err(anyhow::Error::from(error).context("creating window")),
            },
            || std::thread::sleep(std::time::Duration::from_millis(50)),
            || started.elapsed(),
        )
    }
    #[cfg(not(target_os = "android"))]
    {
        use anyhow::Context as _;
        builder.build().context("creating window")
    }
}

/// A wgpu surface with what keeps its native window alive: the surface is declared first, so it is always dropped
/// first (a struct's fields drop in order, on every path, unwinding included).
struct Attached {
    surface: wgpu::Surface<'static>,
    native: Native,
}

/// The wgpu surface of `window`. With `wait` (the window just made), a window without its native surface yet (the app
/// sent to the background as it opened) is waited for until the app is back, instead of failing the start: on Android
/// that is no failure of the graphics.
fn create_surface(
    instance: &wgpu::Instance,
    window: &Window,
    wait: bool,
) -> anyhow::Result<Attached> {
    let native = if wait { Native::of_waiting(window)? } else { Native::of(window)? };
    #[cfg(target_os = "android")]
    let target = native.target();
    // SAFETY: the surface made from these handles is dropped before the window (`Video`'s field order).
    #[cfg(not(target_os = "android"))]
    let target = unsafe { wgpu::SurfaceTargetUnsafe::from_display_and_window(window, window) }
        .map_err(|e| anyhow::anyhow!("getting the window handle: {e}"))?;
    // SAFETY: the callers keep the surface in `Video`, which drops it before the native window and the window (field
    // order).
    let surface = unsafe { instance.create_surface_unsafe(target) }
        .map_err(|e| anyhow::anyhow!("creating the wgpu surface: {e}"))?;
    Ok(Attached { surface, native })
}

/// The kinds of error a surface's configuration is caught for on Android: every kind wgpu reports.
#[cfg(any(target_os = "android", test))]
const CAUGHT: &[wgpu::ErrorFilter] =
    &[wgpu::ErrorFilter::Internal, wgpu::ErrorFilter::Validation, wgpu::ErrorFilter::OutOfMemory];

/// Configures `surface`; whether it worked. On Android an error (the native window abandoned in the background, or out
/// of memory) is caught and logged instead of reaching wgpu's handler, which panics: the surface is then made again
/// later.
fn configure(
    surface: &wgpu::Surface<'static>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> bool {
    #[cfg(target_os = "android")]
    {
        let scopes: Vec<_> = CAUGHT.iter().map(|&filter| device.push_error_scope(filter)).collect();
        surface.configure(device, config);
        // Every scope is popped, the last pushed first.
        let mut failed = None;
        for scope in scopes.into_iter().rev() {
            failed = failed.or(pollster::block_on(scope.pop()));
        }
        if let Some(error) = failed {
            log::warn!("configuring the surface failed (made again later): {error}");
            return false;
        }
        true
    }
    #[cfg(not(target_os = "android"))]
    {
        surface.configure(device, config);
        true
    }
}

/// The graphics of the window, open: on its surface, the format and alpha mode it takes.
struct Gpu {
    instance: wgpu::Instance,
    attached: Attached,
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    alpha_mode: wgpu::CompositeAlphaMode,
}

/// A try of a backend that failed.
#[derive(Debug)]
struct Failed {
    error: anyhow::Error,
    /// The window's native surface was lost meanwhile (Android: the app went to the background as the graphics
    /// opened, and the surface's queries failed on the abandoned window): no failure of the backend.
    lost_window: bool,
}

impl Failed {
    fn of_backend(error: anyhow::Error) -> Self {
        Self { error, lost_window: false }
    }
}

/// The first of `backends` that `attempt` opens (`more`: another is left after it). A try that failed because the
/// window's native surface was lost meanwhile ([`Failed::lost_window`]) is made again on the same backend (its surface
/// waits for the app to be back, [`create_surface`]): it says nothing of the backend, so it is neither the next
/// backend's turn nor a failure of the start. Any other failure is the backend's: the next one is tried.
fn first_that_opens<B: Copy + std::fmt::Debug, T>(
    backends: &[B],
    mut attempt: impl FnMut(B, bool) -> Result<T, Failed>,
) -> anyhow::Result<T> {
    let mut last = anyhow::anyhow!("no graphics backend to try");
    for (index, &backend) in backends.iter().enumerate() {
        let more = index + 1 < backends.len();
        loop {
            match attempt(backend, more) {
                Ok(opened) => return Ok(opened),
                Err(Failed { error, lost_window: true }) => log::info!(
                    "graphics backend {backend:?}: the window's native surface was lost while it opened (in the \
                     background): tried again when the app is back ({error:#})"
                ),
                Err(Failed { error, lost_window: false }) => {
                    log::warn!("graphics backend {backend:?} failed: {error:#}");
                    #[cfg(target_os = "android")]
                    crate::android_report::gpu_failed(format!(
                        "graphics backend {backend:?} failed: {error:#}"
                    ));
                    last = error;
                    break;
                }
            }
        }
    }
    Err(last)
}

/// The graphics on the first backend of [`canvas::backends_to_try`] that works (Android tries Vulkan, then OpenGL
/// ES). A software Vulkan adapter is passed over while another backend is left to try.
fn open_gpu(window: &Window) -> anyhow::Result<Gpu> {
    first_that_opens(&canvas::backends_to_try(), |backend, more| {
        let instance = canvas::instance_on(backend);
        let attached = create_surface(&instance, window, true).map_err(Failed::of_backend)?;
        // A failure from here on is the background's when the native surface is no longer the window's.
        let failed = |error: anyhow::Error| Failed {
            lost_window: !attached.native.is_current(window),
            error,
        };
        #[cfg(all(target_os = "android", debug_assertions))]
        test_pause("slow-adapter");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&attached.surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| failed(anyhow::anyhow!("no wgpu adapter: {e}")))?;
        let info = adapter.get_info();
        log::info!("wgpu adapter {} ({:?}, {:?})", info.name, info.backend, info.device_type);
        #[cfg(target_os = "android")]
        crate::android_report::gpu_opened(format!(
            "{} / {:?} / {} {}",
            info.name, info.backend, info.driver, info.driver_info
        ));
        if more && info.device_type == wgpu::DeviceType::Cpu {
            return Err(Failed::of_backend(anyhow::anyhow!("{} is a software adapter", info.name)));
        }
        let (device, queue) = canvas::device(&adapter).map_err(failed)?;
        #[cfg(all(target_os = "android", debug_assertions))]
        test_pause("slow-device");
        // Empty when the query failed (wgpu passes no error on): on Android, Vulkan's on a window abandoned meanwhile.
        let capabilities = attached.surface.get_capabilities(&adapter);
        let format = pick_format(&capabilities.formats)
            .ok_or_else(|| failed(anyhow::anyhow!("the surface supports no format")))?;
        let alpha_mode = capabilities.alpha_modes.first().copied().unwrap_or_default();
        Ok(Gpu { instance, attached, device, queue, format, alpha_mode })
    })
}

fn window_size(window: &Window) -> (u32, u32) {
    window.size_in_pixels()
}

impl Video {
    /// Opens the window (1440 x 900 points, the size of the design) and sets up wgpu and egui.
    pub fn new(video: &sdl3::VideoSubsystem, title: &str) -> anyhow::Result<Self> {
        let mut builder = video.window(title, 1440, 900);
        builder.high_pixel_density();
        builder.resizable();
        builder.position_centered();
        let mut window = build_window(&builder)?;
        if let Err(e) = window.set_minimum_size(640, 400) {
            log::warn!("setting the minimum window size failed: {e}");
        }

        let Gpu { instance, attached, device, queue, format, alpha_mode } = open_gpu(&window)?;
        let (width, height) = window_size(&window);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            // The main loop paces the frames itself.
            present_mode: present_mode(false),
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![],
        };
        // Not configured (the app went to the background as the window opened): `present` makes it again.
        let surface = configure(&attached.surface, &device, &surface_config).then_some(attached);
        let canvas = Canvas::new(device, queue, format);
        // Text events only while a text field has the keyboard (see `present`).
        let text_input = video.text_input();
        text_input.stop(&window);
        Ok(Self {
            canvas,
            surface_config,
            surface,
            instance,
            cursors: Cursors::default(),
            clipboard: video.clipboard(),
            text_input,
            text_input_on: false,
            minimized: false,
            fullscreen: false,
            reconfigure: Reconfigure::default(),
            started: Instant::now(),
            window,
        })
    }

    /// Windows keeps reporting the last size while minimised, so track it explicitly
    /// (`WindowEvent::Minimized` / `WindowEvent::Restored`).
    pub fn set_minimized(&mut self, minimized: bool) {
        self.minimized = minimized;
    }

    /// The app goes to the background (Android): the window's native surface is about to be destroyed, so the wgpu
    /// surface is dropped now and nothing is drawn until [`Video::resume`].
    #[cfg(target_os = "android")]
    pub fn suspend(&mut self) {
        self.surface = None;
        self.minimized = true;
    }

    /// The app is back in the foreground: the wgpu surface is made again on the window's new native surface
    /// (`present` tries again on its own if the window has none yet).
    #[cfg(target_os = "android")]
    pub fn resume(&mut self) {
        self.minimized = false;
        self.recreate_surface();
    }

    /// Makes the wgpu surface on the window as it is now and configures it; whether it worked.
    fn recreate_surface(&mut self) -> bool {
        let (width, height) = window_size(&self.window);
        if !should_render(width, height) {
            return false;
        }
        match create_surface(&self.instance, &self.window, false) {
            Ok(attached) => {
                self.surface_config.width = width;
                self.surface_config.height = height;
                if !configure(&attached.surface, self.canvas.device(), &self.surface_config) {
                    return false;
                }
                self.surface = Some(attached);
                self.reconfigure = Reconfigure::default();
                log::info!("the surface is made again ({width} x {height})");
                true
            }
            Err(e) => {
                log::warn!(
                    "making the surface again failed (tried again at the next frame): {e:#}"
                );
                false
            }
        }
    }

    /// Window or fullscreen (the desktop's own mode, so the picture keeps its aspect in the window's size).
    pub fn set_fullscreen(&mut self, on: bool) {
        if self.fullscreen == on {
            return;
        }
        match self.window.set_fullscreen(on) {
            Ok(()) => self.fullscreen = on,
            Err(e) => log::warn!("setting fullscreen {on} failed: {e}"),
        }
    }

    /// V-sync on or off: the surface is configured again at the next present.
    pub fn set_vsync(&mut self, on: bool) {
        let mode = present_mode(on);
        if self.surface_config.present_mode != mode {
            self.surface_config.present_mode = mode;
            self.reconfigure.request();
        }
    }

    pub fn set_title(&mut self, title: &str) {
        if let Err(e) = self.window.set_title(title) {
            log::warn!("setting the window title failed: {e}");
        }
    }

    pub fn handle_resize(&mut self) {
        let (width, height) = window_size(&self.window);
        if should_render(width, height)
            && (width, height) != (self.surface_config.width, self.surface_config.height)
        {
            self.surface_config.width = width;
            self.surface_config.height = height;
            // Configured in an error scope (Android): the system may have destroyed the native window before the main
            // thread saw the app go to the background; the surface is then made again.
            if let Some(attached) = &self.surface
                && !configure(&attached.surface, self.canvas.device(), &self.surface_config)
            {
                self.surface = None;
            }
        }
    }

    /// How SDL's coordinates relate to egui's points in this window now.
    pub fn geometry(&self) -> Geometry {
        Geometry {
            pixel_density: self.window.pixel_density(),
            pixels_per_point: self.window.display_scale().max(0.1),
        }
    }

    /// The window's area in egui points: what the touch controls lay out in.
    pub fn screen_points(&self) -> egui::Rect {
        let (width, height) = window_size(&self.window);
        self.geometry().screen_rect(width, height)
    }

    /// Seconds since the window opened: the clock of the interface.
    pub fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Whether a text field of the interface has the keyboard (as of the last frame).
    pub fn text_focus(&self) -> bool {
        self.canvas.context().text_edit_focused()
    }

    /// The system clipboard's text.
    pub fn clipboard_text(&self) -> Option<String> {
        self.clipboard.clipboard_text().ok().filter(|text| !text.is_empty())
    }

    /// Feeds an SDL event to the interface.
    pub fn handle_event(&self, input: &mut InputAdapter, event: &Event) {
        input.handle(event, self.geometry(), self.text_focus(), || self.clipboard_text());
    }

    /// Draws one window frame: the game views (each of `views` as a texture for `ui` to place, in
    /// order) and the interface `ui` builds from the events in `input`. Skipped while the window is
    /// minimised.
    pub fn present(
        &mut self,
        views: &[GameView<'_>],
        input: &mut InputAdapter,
        ui: impl FnMut(&mut egui::Ui, &[GameImage]),
    ) -> anyhow::Result<()> {
        let outcome = self.draw(views, input, ui)?;
        if note(outcome) {
            // Android: nothing shown for 3 s in the foreground: the surface is made again, once a start
            // (`android_report::present_outcome`).
            self.surface = None;
            #[cfg(all(target_os = "android", debug_assertions))]
            crate::android_report::test_surface_rebuilt();
        }
        Ok(())
    }

    /// [`Video::present`]'s frame: what it came to.
    fn draw(
        &mut self,
        views: &[GameView<'_>],
        input: &mut InputAdapter,
        ui: impl FnMut(&mut egui::Ui, &[GameImage]),
    ) -> anyhow::Result<Outcome> {
        let size = window_size(&self.window);
        if self.minimized || !should_render(size.0, size.1) {
            return Ok(Outcome::Skipped);
        }
        if self.surface.is_none() && !self.recreate_surface() {
            return Ok(Outcome::Skipped);
        }
        let raw =
            input.take_raw_input(self.geometry(), size, self.now(), self.canvas.max_texture_side());
        let pass = self.canvas.run(raw, views, ui);
        self.handle_platform_output(&pass.platform);
        // The last texture was presented, so none is alive: the surface may be configured now (in an error scope, as
        // in `handle_resize`).
        if self.reconfigure.take()
            && let Some(attached) = &self.surface
            && !configure(&attached.surface, self.canvas.device(), &self.surface_config)
        {
            self.surface = None;
        }
        let Some(surface) = self.surface.as_ref().map(|attached| &attached.surface) else {
            self.canvas.discard(pass);
            return Ok(Outcome::Skipped);
        };
        // Test builds only: the first picture's present fails, so the run returns with an error at its first frame.
        #[cfg(all(target_os = "android", debug_assertions))]
        if std::env::var("SSD_TEST_FAIL").as_deref() == Ok("present-error") {
            self.canvas.discard(pass);
            anyhow::bail!("the wgpu surface failed validation (SSD_TEST_FAIL=present-error)")
        }
        // Test builds only: every present comes to nothing, as a lost surface (`android_report::test_black_screen`).
        #[cfg(all(target_os = "android", debug_assertions))]
        if crate::android_report::test_black_screen() {
            surface.configure(self.canvas.device(), &self.surface_config);
            self.canvas.discard(pass);
            return Ok(Outcome::Lost);
        }
        let surface_texture = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                // Drawn and presented as it is; the surface adapts to the window before the next.
                self.reconfigure.request();
                texture
            }
            refused @ (wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost) => {
                let configured = configure(surface, self.canvas.device(), &self.surface_config);
                self.canvas.discard(pass);
                if !configured {
                    // Its native window is gone: made again from the window's next one.
                    self.surface = None;
                }
                return Ok(if matches!(refused, wgpu::CurrentSurfaceTexture::Lost) {
                    Outcome::Lost
                } else {
                    Outcome::Outdated
                });
            }
            refused @ (wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded) => {
                self.canvas.discard(pass);
                return Ok(if matches!(refused, wgpu::CurrentSurfaceTexture::Timeout) {
                    Outcome::Timeout
                } else {
                    Outcome::Occluded
                });
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                self.canvas.discard(pass);
                anyhow::bail!("the wgpu surface failed validation")
            }
        };
        let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.canvas.paint(pass, &view, [self.surface_config.width, self.surface_config.height]);
        self.canvas.queue().present(surface_texture);
        // Android: the start has drawn its first picture (`android_report::first_frame`).
        Ok(Outcome::Shown)
    }

    /// Draws the picture of the window, as [`Video::present`] does but into a texture of the window's
    /// size and with the interface `ui` builds (the game alone, for a screenshot), and returns it:
    /// width, height and the RGBA pixels, top row first. The window itself is not touched.
    ///
    /// # Errors
    ///
    /// The window is not drawn now (minimised), or the pixels cannot be read back from the GPU.
    pub fn capture(
        &mut self,
        views: &[GameView<'_>],
        ui: impl FnMut(&mut egui::Ui, &[GameImage]),
    ) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        let (width, height) = window_size(&self.window);
        anyhow::ensure!(!self.minimized && should_render(width, height), "the window is not drawn");
        let format = self.surface_config.format;
        let target = self.canvas.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        // No pointer and no keys: the input is the empty one, at the window's geometry.
        let mut input = InputAdapter::default();
        let raw = input.take_raw_input(
            self.geometry(),
            (width, height),
            self.now(),
            self.canvas.max_texture_side(),
        );
        let pass = self.canvas.run(raw, views, ui);
        self.canvas.paint(pass, &view, [width, height]);
        let mut pixels = crate::snapshot::read_texture(
            self.canvas.device(),
            self.canvas.queue(),
            &target,
            width,
            height,
        )?;
        let bgra =
            matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        for pixel in pixels.as_chunks_mut::<4>().0 {
            if bgra {
                pixel.swap(0, 2);
            }
            pixel[3] = 255;
        }
        Ok((width, height, pixels))
    }

    /// Carries out what egui asked of the system: cursor, clipboard, text input.
    fn handle_platform_output(&mut self, output: &egui::PlatformOutput) {
        self.cursors.show(output.cursor_icon);
        for command in &output.commands {
            if let egui::OutputCommand::CopyText(text) = command
                && let Err(e) = self.clipboard.set_clipboard_text(text)
            {
                log::warn!("copying to the clipboard failed: {e}");
            }
        }
        let wants_text = self.canvas.context().text_edit_focused();
        if wants_text != self.text_input_on {
            self.text_input_on = wants_text;
            if wants_text {
                self.text_input.start(&self.window);
            } else {
                self.text_input.stop(&self.window);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vsync_picks_the_presentation_mode() {
        assert_eq!(
            present_mode(false),
            wgpu::PresentMode::AutoNoVsync,
            "off is what the window always did"
        );
        assert_eq!(present_mode(true), wgpu::PresentMode::AutoVsync);
    }

    #[test]
    fn zero_sized_window_is_not_rendered() {
        assert!(!should_render(0, 0));
        assert!(!should_render(800, 0));
        assert!(should_render(1, 1));
    }

    #[test]
    fn the_surface_format_is_a_linear_one_the_surface_offers() {
        use wgpu::TextureFormat as F;
        assert_eq!(pick_format(&[F::Bgra8UnormSrgb, F::Bgra8Unorm]), Some(F::Bgra8Unorm));
        assert_eq!(pick_format(&[F::Rgba16Float, F::Rgba8Unorm]), Some(F::Rgba8Unorm));
        assert_eq!(pick_format(&[F::Rgb10a2Unorm, F::Rgba8UnormSrgb]), Some(F::Rgb10a2Unorm));
        assert_eq!(pick_format(&[F::Bgra8UnormSrgb, F::Rgba8UnormSrgb]), Some(F::Bgra8UnormSrgb));
        assert_eq!(pick_format(&[]), None, "a surface with no format");
    }

    #[test]
    fn a_suboptimal_surface_is_configured_again_once_then_not_until_asked_again() {
        let mut reconfigure = Reconfigure::default();
        assert!(!reconfigure.take(), "nothing due at first");
        reconfigure.request();
        assert!(reconfigure.take(), "due after a suboptimal acquire");
        assert!(!reconfigure.take(), "taken: not due again");
        reconfigure.request();
        reconfigure.request();
        assert!(reconfigure.take());
        assert!(!reconfigure.take(), "requests do not stack");
    }

    #[test]
    fn configuring_catches_every_kind_of_error() {
        // Exhaustive: a kind wgpu adds stops this from compiling until it is caught.
        fn index(filter: wgpu::ErrorFilter) -> usize {
            match filter {
                wgpu::ErrorFilter::Internal => 0,
                wgpu::ErrorFilter::Validation => 1,
                wgpu::ErrorFilter::OutOfMemory => 2,
            }
        }
        let mut caught = [false; 3];
        for &filter in CAUGHT {
            caught[index(filter)] = true;
        }
        assert_eq!(caught, [true; 3], "{CAUGHT:?}");
    }

    #[test]
    fn a_window_waits_for_its_native_surface_however_long_the_app_is_away() {
        // In the background 40 minutes (frozen there, say) between two looks: still waited for, never a failure.
        let (mut tries, clock) = (0, std::cell::Cell::new(std::time::Duration::ZERO));
        let got = wait_for_native(
            "test",
            || {
                tries += 1;
                Ok((tries == 4).then_some("window"))
            },
            || clock.set(clock.get() + std::time::Duration::from_mins(40)),
            || clock.get(),
        );
        assert_eq!(got.unwrap(), "window");
        assert_eq!(tries, 4);
        // Another error is no waiting matter.
        let refused: anyhow::Result<()> =
            wait_for_native("test", || anyhow::bail!("refused"), || {}, || clock.get());
        assert!(refused.is_err());
    }

    /// `first_that_opens` over Vulkan then OpenGL ES, each try failing as `fails` says (`None`: it opens); the
    /// backends tried in turn and what opened.
    fn open(
        fails: &mut Vec<(&'static str, Option<bool>)>,
    ) -> (Vec<&'static str>, anyhow::Result<&'static str>) {
        let mut tried = Vec::new();
        let opened = first_that_opens(&["vulkan", "gl"], |backend, _more| {
            tried.push(backend);
            let at = fails.iter().position(|(b, _)| *b == backend).expect("a try planned");
            match fails.remove(at).1 {
                None => Ok(backend),
                Some(lost_window) => {
                    Err(Failed { error: anyhow::anyhow!("{backend} failed"), lost_window })
                }
            }
        });
        (tried, opened)
    }

    #[test]
    fn a_backend_whose_window_was_lost_as_it_opened_is_tried_again_not_passed_over() {
        // Home as Vulkan made its device: its surface's formats came back empty on the abandoned window.
        let (tried, opened) =
            open(&mut vec![("vulkan", Some(true)), ("vulkan", Some(true)), ("vulkan", None)]);
        assert_eq!(tried, ["vulkan", "vulkan", "vulkan"]);
        assert_eq!(opened.unwrap(), "vulkan", "never OpenGL ES for the background");
    }

    #[test]
    fn a_backend_that_fails_with_its_window_there_gives_way_to_the_next() {
        let (tried, opened) = open(&mut vec![("vulkan", Some(false)), ("gl", None)]);
        assert_eq!(tried, ["vulkan", "gl"]);
        assert_eq!(opened.unwrap(), "gl");
        let (tried, opened) = open(&mut vec![("vulkan", Some(false)), ("gl", Some(false))]);
        assert_eq!(tried, ["vulkan", "gl"]);
        assert_eq!(opened.unwrap_err().to_string(), "gl failed", "the last backend's failure");
    }

    #[test]
    fn cursors_follow_the_icon_egui_asks_for() {
        assert_eq!(CursorKind::of(egui::CursorIcon::Default), CursorKind::Arrow);
        assert_eq!(CursorKind::of(egui::CursorIcon::PointingHand), CursorKind::Hand);
        assert_eq!(CursorKind::of(egui::CursorIcon::Text), CursorKind::IBeam);
        assert_eq!(CursorKind::of(egui::CursorIcon::Wait), CursorKind::Arrow);
        let mut seen = [false; 3];
        for kind in [CursorKind::Arrow, CursorKind::Hand, CursorKind::IBeam] {
            seen[kind.index()] = true;
        }
        assert_eq!(seen, [true; 3], "each kind has a slot of its own");
    }
}
