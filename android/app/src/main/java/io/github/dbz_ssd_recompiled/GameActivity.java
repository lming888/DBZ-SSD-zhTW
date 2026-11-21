package io.github.dbz_ssd_recompiled;

import android.app.ActivityManager;
import android.app.ApplicationExitInfo;
import android.content.pm.ApplicationInfo;
import android.media.AudioManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.view.Display;
import android.view.HapticFeedbackConstants;
import android.view.Surface;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;

import org.libsdl.app.SDLActivity;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.List;

/**
 * The game's activity: SDL's activity, which loads libSDL3.so and libmain.so and calls SDL_main.
 */
public class GameActivity extends SDLActivity {
    /** A light haptic tick under the finger (the touch controls); sent by the native side. */
    private static final int HAPTIC = 0x8002;
    /** Test builds only (a debuggable APK): the activity's thread stops answering, for an ANR (native SSD_TEST_FAIL=anr). */
    private static final int TEST_HANG = 0x8003;
    /** The file of the last processes' ends (Android 11+), read by the native side (crates/ssd-app/src/exit_info.rs). */
    private static final String EXITS_FILE = "process-exits.txt";
    /** The most of a trace (a native crash's tombstone, an ANR's stacks) kept for the report. */
    private static final int TRACE_MAX = 512 * 1024;
    /**
     * The native side's commands: count the foreground time from now (the graphics of the setup screens or of the game
     * begin to open), stop (the first picture, or a dialog of the native side's is up).
     */
    private static final int COUNT_START = 0x8004;
    private static final int COUNT_STOP = 0x8005;
    /** The foreground time counted, read by the native side at the next start (crates/ssd-app/src/launch_guard.rs). */
    private static final String FOREGROUND_FILE = "foreground.txt";
    /** How often the count is written while in the foreground, and up to which total (past it nothing changes). */
    private static final long COUNT_WRITE_MS = 2000;
    private static final long COUNT_MAX_MS = 30_000;

    /** Counting the foreground time, the milliseconds counted before the last resume, and when that was (-1: paused). */
    private boolean counting;
    private long countedMs;
    private long resumedAt = -1;
    private final Handler countHandler = new Handler(Looper.getMainLooper());
    private final Runnable countTick = this::countTick;
    /** The game's frame rate (the SNES's 60.1, which a 60 Hz screen shows). */
    private static final float GAME_RATE = 60f;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // Before SDL starts the game's thread, which reads it.
        writeProcessExits();
        super.onCreate(savedInstanceState);
        setVolumeControlStream(AudioManager.STREAM_MUSIC);
        // The screen stays on while the game is in front, and the picture reaches the edges (cutouts included).
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        if (Build.VERSION.SDK_INT >= 28) {
            WindowManager.LayoutParams attrs = getWindow().getAttributes();
            attrs.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            getWindow().setAttributes(attrs);
        }
        hideSystemBars();
        requestRefreshRate();
    }

    @Override
    protected void onResume() {
        super.onResume();
        resumedAt = SystemClock.uptimeMillis();
        if (counting) {
            countHandler.removeCallbacks(countTick);
            countHandler.postDelayed(countTick, COUNT_WRITE_MS);
        }
    }

    @Override
    protected void onPause() {
        if (resumedAt >= 0 && counting) {
            countedMs += SystemClock.uptimeMillis() - resumedAt;
            writeForeground(countedMs);
        }
        resumedAt = -1;
        countHandler.removeCallbacks(countTick);
        super.onPause();
    }

    /** While counting in the foreground: the total so far, written every COUNT_WRITE_MS up to COUNT_MAX_MS. */
    private void countTick() {
        if (!counting || resumedAt < 0) {
            return;
        }
        long total = countedMs + SystemClock.uptimeMillis() - resumedAt;
        writeForeground(total);
        if (total < COUNT_MAX_MS) {
            countHandler.postDelayed(countTick, COUNT_WRITE_MS);
        }
    }

    /** Writes this process's foreground time: pid, then milliseconds. */
    private void writeForeground(long ms) {
        try (OutputStream out = new FileOutputStream(new File(getFilesDir(), FOREGROUND_FILE))) {
            out.write((android.os.Process.myPid() + "\t" + ms + "\n").getBytes(StandardCharsets.UTF_8));
        } catch (Throwable error) {
            android.util.Log.w("ssd", "the foreground time was not written: " + error);
        }
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
            // Again: a fold or an unfold is another display, with its own modes.
            requestRefreshRate();
        }
    }

    /** Immersive full screen: the system bars are hidden and come back with a swipe from the edge. */
    private void hideSystemBars() {
        Window window = getWindow();
        if (Build.VERSION.SDK_INT >= 30) {
            // The flags below are ignored by recent systems; the insets controller is their replacement.
            window.setDecorFitsSystemWindows(false);
            WindowInsetsController controller = window.getInsetsController();
            if (controller != null) {
                controller.hide(WindowInsets.Type.statusBars() | WindowInsets.Type.navigationBars());
                controller.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
            return;
        }
        window.getDecorView().setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                        | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                        | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                        | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_FULLSCREEN);
    }

    /**
     * Asks the screen for a refresh rate the game's 60 frames fit: the lowest mode of the current resolution that is a
     * multiple of 60 Hz, and the surface's frame rate. An adaptive (LTPO) panel otherwise drops to 24 Hz or so while
     * nothing touches it, and the game, whose frames wait for the screen, slows down with it (and its sound breaks up).
     */
    private void requestRefreshRate() {
        Display display = getWindowManager().getDefaultDisplay();
        Display.Mode current = display.getMode();
        Display.Mode best = null;
        for (Display.Mode mode : display.getSupportedModes()) {
            if (mode.getPhysicalWidth() != current.getPhysicalWidth()
                    || mode.getPhysicalHeight() != current.getPhysicalHeight()) {
                continue;
            }
            float multiple = mode.getRefreshRate() / GAME_RATE;
            boolean fits = multiple >= 0.98f && Math.abs(multiple - Math.round(multiple)) < 0.02f;
            if (fits && (best == null || mode.getRefreshRate() < best.getRefreshRate())) {
                best = mode;
            }
        }
        WindowManager.LayoutParams attrs = getWindow().getAttributes();
        if (best != null) {
            attrs.preferredDisplayModeId = best.getModeId();
        }
        attrs.preferredRefreshRate = best != null ? best.getRefreshRate() : GAME_RATE;
        getWindow().setAttributes(attrs);
        if (Build.VERSION.SDK_INT >= 30 && mSurface != null) {
            Surface surface = mSurface.getHolder().getSurface();
            if (surface != null && surface.isValid()) {
                surface.setFrameRate(GAME_RATE, Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE);
            }
        }
    }

    /**
     * Writes how the app's last processes ended (Android 11+: the system's ApplicationExitInfo), one a line:
     * pid, reason, status, time (ms since 1970), the trace's file (a native crash's tombstone or an ANR's stacks, kept
     * beside it) and the system's description. The native side matches the line of the start that did not draw its first
     * picture (its pid is in its stage file) and only counts a crash, not the user or the system ending the app.
     */
    private void writeProcessExits() {
        // Only after a start that left its stage file (crates/ssd-app/src/android_report.rs): otherwise nothing reads
        // them, and the traces are not read on the activity's thread at every start.
        if (Build.VERSION.SDK_INT < 30 || !new File(getFilesDir(), "launch-stage.txt").exists()) {
            return;
        }
        try {
            // The traces of the last start's list are written again below: the older ones go.
            File[] old = getFilesDir().listFiles((dir, name) -> name.startsWith("process-exit-") && name.endsWith(".trace"));
            if (old != null) {
                for (File file : old) {
                    file.delete();
                }
            }
            ActivityManager am = (ActivityManager) getSystemService(ACTIVITY_SERVICE);
            List<ApplicationExitInfo> exits = am.getHistoricalProcessExitReasons(getPackageName(), 0, 5);
            StringBuilder text = new StringBuilder();
            for (ApplicationExitInfo exit : exits) {
                String trace = "";
                int reason = exit.getReason();
                if (reason == ApplicationExitInfo.REASON_CRASH_NATIVE || reason == ApplicationExitInfo.REASON_ANR) {
                    trace = "process-exit-" + exit.getPid() + ".trace";
                    if (!saveTrace(exit, new File(getFilesDir(), trace))) {
                        trace = "";
                    }
                }
                String description = exit.getDescription() == null ? "" : exit.getDescription();
                text.append(exit.getPid()).append('\t').append(reason).append('\t').append(exit.getStatus())
                        .append('\t').append(exit.getTimestamp()).append('\t').append(trace).append('\t')
                        .append(description.replace('\t', ' ').replace('\n', ' ').replace('\r', ' ')).append('\n');
            }
            try (OutputStream out = new FileOutputStream(new File(getFilesDir(), EXITS_FILE))) {
                out.write(text.toString().getBytes(StandardCharsets.UTF_8));
            }
        } catch (Throwable error) {
            android.util.Log.w("ssd", "the last processes' ends were not read: " + error);
        }
    }

    /** Copies the trace of `exit` (at most TRACE_MAX bytes) to `file`; whether there was one. */
    private static boolean saveTrace(ApplicationExitInfo exit, File file) {
        try (InputStream in = exit.getTraceInputStream()) {
            if (in == null) {
                return false;
            }
            try (OutputStream out = new FileOutputStream(file)) {
                byte[] block = new byte[16384];
                int total = 0;
                int read;
                while (total < TRACE_MAX && (read = in.read(block, 0, Math.min(block.length, TRACE_MAX - total))) > 0) {
                    out.write(block, 0, read);
                    total += read;
                }
            }
            return true;
        } catch (Throwable error) {
            android.util.Log.w("ssd", "a trace was not read: " + error);
            return false;
        }
    }

    @Override
    protected boolean onUnhandledMessage(int command, Object param) {
        if (command == TEST_HANG && (getApplicationInfo().flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0) {
            // Test builds only: the activity's thread sleeps, so the system finds the app not responding.
            android.util.Log.w("ssd", "TEST_HANG: the activity's thread sleeps 60 s");
            try {
                Thread.sleep(60_000);
            } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
            }
            return true;
        }
        if (command == COUNT_START) {
            counting = true;
            countedMs = 0;
            if (resumedAt >= 0) {
                resumedAt = SystemClock.uptimeMillis();
            }
            writeForeground(0);
            countHandler.removeCallbacks(countTick);
            countHandler.postDelayed(countTick, COUNT_WRITE_MS);
            return true;
        }
        if (command == COUNT_STOP) {
            counting = false;
            countHandler.removeCallbacks(countTick);
            return true;
        }
        if (command == HAPTIC) {
            // The system's keyboard-tap feedback: light, and it needs no permission.
            getWindow().getDecorView().performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP);
            return true;
        }
        return false;
    }
}
