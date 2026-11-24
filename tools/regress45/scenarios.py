"""scenarios.py -- the runs and scenarios of the regression suite (tools/regress45/run.py).

A RUN is one scan.py pass: a script, the frames drawn and checked, the host writes, the controls (edited tables).
A SCENARIO is one assertion about a run (several share a run):
  kind 'fixed'  a bug that was fixed (must PASS);
  kind 'guard'  a screen that is clean today (must PASS: it catches a change that breaks it);
  kind 'known'  a bug not fixed yet (expected failure: XFAIL; when it passes, XPASS: take the mark off, make it
                'fixed').
Its checks:
  no(rules, (first, last)..., key=REGEX)  no frame with a hit of these rules (in these frames, on a page key matching
                                          REGEX). A 'fixed' or 'guard' scenario does not count the hits the 'known'
                                          scenarios of the same run claim (their own `no` checks), nor its `allow`.
  probe(...)                              a pixel probe of scan.py (see its docstring): no bad frame.
Boxes are pixels of the 2x picture (744x478); the evidence PNGs of the scan put a 36-pixel header above it.

Rule groups: ENGLISH = English letters back where the overlay wrote Chinese (REVERT, INPAPER); WRONG = Chinese drawn
on a window that is not its page's (WRONGPAGE, MASKED).
"""

ENGLISH = ['REVERT', 'INPAPER']
WRONG = ['WRONGPAGE', 'MASKED']
CLEAN = ENGLISH + WRONG + ['UNDRAWN']

# Porunga (item 0F) in the battle's item list with 150 points (`$7E2BA0` its status word)
PORUNGA = ['4699:7E0100=0F80', '4699:7E01B2=01', '4699:7E2BA0=0096']
PORUNGA92 = ['4699:7E0100=0F92', '4699:7E01B2=01', '4699:7E2BA0=0096']
# one item (id ID, attribute 80) in the battle's list; Goku and Piccolo at KI 10 and BP 1 (the item scenes)
def ITEM(code):
    return [f'4699:7E0100={code}', '4699:7E01B2=01', '4699:7E0B03=0A00', '4699:7E0B23=0A00', '4699:7E0B05=0100',
            '4699:7E0B25=0100']
# Goku and Piccolo at level 20, KI 999 (battle menus, special moves)
LV20 = ['4699:7E0B02=14', '4699:7E0B05=E703', '4699:7E0B0C=E703', '4699:7E0B22=14', '4699:7E0B25=E703',
        '4699:7E0B2C=E703']
# Baba's shop: the town (landing spot 7, map $30) by the entity-touch state of 03:970A, 999 zeni
SHOP = ['3700:7E0024=01', '3700:7E0025=03', '3700:7E0716=A1', '3700:7E0D87=01', '3700:7E01BD=07',
        '3700:7E01B4=E703']
GRAV = ['2304:7E01D8=50', '2304:7E1648=80']       # the gravity room scene (the old grav.script pokes)
# The training ground (landing spot 8), teleported in as Baba's shop is (SHOP)
TRAIN = ['3700:7E0024=01', '3700:7E0025=03', '3700:7E0716=A1', '3700:7E0D87=00', '3700:7E01BD=08']

# Save states made before the runs (xtask shots SCRIPT --upto N --state-out; 'sram' and 'wram' as a run's)
STATES = {
    'ssjf_8520': {'script': 'replay:secret-ssj-frieza', 'upto': 8520, 'hooks': True},
    'fusion_2990': {'script': 'replay:secret-piccolo-dende-fusion', 'upto': 2990, 'hooks': True},
    # (Porunga's wish window after `str4:0x03`, whose `und` and `s.▼` stay right of the window: a state loaded there)
    'porunga_9600': {'script': 'replay:secret-nail-porunga', 'upto': 9600, 'hooks': True},
    # (the trip to Namek: `str3:0x4C` "34 days later…" stays up while the story transition stops the message, frames
    # 4887-5338, the engine naming no page; a state loaded there)
    'namek_trip_4900': {'script': 'scenes/story_names/cont_mash.txt', 'upto': 4900, 'hooks': True, 'sram': 'grav',
                        'wram': ['2304:7E121F=07', '2304:7E0024=87']},
}

# The story transition after the Saiyans (`transition_target` 7: Vegeta leaves, the flight, the trip to Namek), run on
# the gravity-room save by writing it and `game_mode` $87 at 2304 (crafted; docs/re/data/title-chapters.md 2)
NAMEK_TRIP = ['2304:7E121F=07', '2304:7E0024=87']

# script: 'scenes/...' (a timeline of this folder), 'replay:NAME' (the input replay, rebased), or a builtin name.
# hooks defaults to True (the Enhanced app's hook table, as the game runs).
RUNS = {
    # ---- Porunga in battle: wishes 1 then 3, 3 then 1, 2 only
    'porunga_13': dict(script='scenes/porunga/tl13.txt', lo=4500, hi=7176, wram=PORUNGA),
    'porunga_13_nohooks': dict(script='scenes/porunga/tl13.txt', lo=4500, hi=7176, wram=PORUNGA, hooks=False),
    'porunga_31': dict(script='scenes/porunga/tl31.txt', lo=4500, hi=6776, wram=PORUNGA),
    'porunga_2': dict(script='scenes/porunga/tl2.txt', lo=4500, hi=6232, wram=PORUNGA92,
                      controls={'no_wishlist': {'drop': ['str7:0x00', 'help:15']}}),
    # ---- battle items
    'bi_grandpa': dict(script='scenes/battle_items/generic_use.txt', lo=4500, hi=5100, wram=ITEM('0780'),
                       controls={'no_help7': {'drop': ['help:7']}}),
    'bi_yajirobe': dict(script='scenes/battle_items/generic_use.txt', lo=4500, hi=5100, wram=ITEM('1280'),
                        controls={'no_help18': {'drop': ['help:18']}}),
    'bi_piccolo': dict(script='scenes/battle_items/generic_use.txt', lo=4500, hi=5200, wram=ITEM('1D80'),
                       controls={'no_help29': {'drop': ['help:29']}}),
    'bi_drop_yes': dict(script='scenes/battle_items/drop_yes.txt', lo=4500, hi=5300,
                        wram=['4699:7E0100=00C801C8', '4699:7E01B2=02'],
                        controls={'no_help1': {'drop': ['help:1']}}),
    'bi_drop_porunga': dict(script='scenes/battle_items/drop_yes.txt', lo=4500, hi=5300,
                            wram=['4699:7E0100=00C80F92', '4699:7E01B2=02', '4699:7E2BA2=0096']),
    'bi_obj27': dict(script='scenes/battle_items/generic_use.txt', lo=4500, hi=5650, wram=ITEM('1B84')),
    'bi_help30': dict(script='scenes/battle_items/generic_use.txt', lo=4500, hi=4900,
                      wram=['4699:7E0100=1E41', '4699:7E01B2=01']),
    # (object 1F, help:31: three lines of Chinese, the English one `.`; the control draws the English page)
    'bi_help31': dict(script='scenes/battle_items/generic_use.txt', lo=4500, hi=4900,
                      wram=['4699:7E0100=1F41', '4699:7E01B2=01'], controls={'no_help31': {'drop': ['help:31']}}),
    'bi_p1b_long': dict(script='scenes/battle_items/p1b_long.txt', lo=4500, hi=7376, wram=PORUNGA92),
    'bi_kami': dict(script='scenes/battle_items/kami.txt', lo=4500, hi=5572,
                    wram=['4699:7E0100=0080', '4699:7E01B2=01']),
    # ---- battle commands, cards, special moves
    'battle': dict(script='battle', lo=2300, hi=7200, controls={'no_battle89': {'drop': ['battle:89']}}),
    'bm_run1': dict(script='scenes/battle_menus/run1.txt', lo=4500, hi=5000),
    'bm_menu1': dict(script='scenes/battle_menus/menu1.txt', lo=4500, hi=5900),
    'bm_itemcmd': dict(script='scenes/battle_menus/itemcmd.txt', lo=4500, hi=5000, wram=PORUNGA),
    'bm_sp1': dict(script='scenes/battle_menus/sp1.txt', lo=4500, hi=5400, wram=LV20),
    'bm_lvlup': dict(script='scenes/battle_menus/lvlup.txt', lo=6700, hi=6990,
                     wram=['4699:7E0B0F=FFFF00', '4699:7E0B2F=FFFF00']),
    'bm_sp_needki': dict(script='scenes/battle_menus/sp_needki.txt', lo=4500, hi=5700,
                         wram=['4699:7E0B02=14', '4699:7E0B05=0100', '4699:7E0B0C=E703']),
    # ---- story battles (replays)
    'bs_ssjf': dict(script='replay:secret-ssj-frieza', lo=8400, hi=9000),
    'bs_ssjf_run': dict(script='scenes/battle_story/ssjf_cmd_run.txt', state='ssjf_8520', lo=8521, hi=9200),
    'bs_ssjf_menu': dict(script='scenes/battle_story/ssjf_cmd_menu.txt', state='ssjf_8520', lo=8521, hi=9500),
    'bs_forced': dict(script='replay:secret-frieza-forced-battle', lo=2400, hi=3100),
    'bs_jeice': dict(script='replay:secret-ginyu-jeice-burter', lo=1700, hi=2400),
    # ---- map dialogue, story pages
    'sd_fusion': dict(script='replay:secret-piccolo-dende-fusion', lo=2300, hi=2750),
    'sd_porunga': dict(script='replay:secret-nail-porunga', lo=8700, hi=10110),
    # (Porunga's opening page and Vegeta's pages after the wishes: a `▼` cell the English line ran past the window)
    'sd_porunga_arrow': dict(script='replay:secret-nail-porunga', lo=9300, hi=10225),
    # (started cold from a state: the overlay never saw the page that wrote the letters right of the wish window)
    'sd_porunga_cold': dict(script='replay:secret-nail-porunga', state='porunga_9600', lo=9601, hi=9700),
    'sd_zfighters': dict(script='replay:secret-namek-zfighters', lo=2000, hi=2850),
    # (the trip to Namek's captions, `str3:0x4B` to `str3:0x4C/p2`: frame by frame, and from a state loaded while the
    # transition stops the message after "34 days later…")
    'st_namek_trip': dict(script='scenes/story_names/cont_mash.txt', sram='grav', lo=4600, hi=5400, wram=NAMEK_TRIP),
    'st_namek_trip_cold': dict(script='scenes/story_names/cont_mash.txt', state='namek_trip_4900', lo=4901,
                               hi=5400),
    # (Dende's fusion choice, str2:0x0EE: the replay picks it at once, so from a state before it, no key: the
    # choice waits with its cursor)
    'sd_fusion_choice': dict(script='scenes/story_choice/idle.txt', state='fusion_2990', lo=2991, hi=3300),
    # ---- name slots the English page lacks (checked against the Japanese ROM): scenes started on the gravity-room save
    # by a write of scene_id at 2304 (crafted, scenes/story_names/cont_mash.txt); the control writes the name out
    'sd_names_s51': dict(script='scenes/story_names/cont_mash.txt', sram='grav', lo=2530, hi=2612,
                         wram=['2304:7E01D8=51', '2304:7E1648=80'],
                         controls={'nail_written': {'set': {'str3:0x67': 'Ｚ戰士們決定\\n和尼爾\\n分開……。'}}}),
    'sd_names_s2D': dict(script='scenes/story_names/cont_mash.txt', sram='grav', lo=3530, hi=3612,
                         wram=['2304:7E01D8=2D', '2304:7E1648=80'],
                         controls={'dende_cargo_written': {'set': {'str3:0x67': 'Ｚ戰士們決定\\n和丹丹　卡爾克\\n分開……。'}}}),
    'sd_names_s1E': dict(script='scenes/story_names/cont_mash.txt', sram='grav', lo=2450, hi=2490,
                         wram=['2304:7E01D8=1E', '2304:7E1648=80']),
    # (scene 47, the trap: a second party record, Piccolo at 1 HP, so that effect $40 names him dead)
    'sd_names_s47': dict(script='scenes/story_names/cont_mash.txt', sram='grav', lo=2540, hi=2570,
                         wram=['2304:7E01D8=47', '2304:7E1648=80',
                               '2304:7E0B20=C0010101000C00A0010050000C000100000000000000001E0000000100000000'],
                         controls={'piccolo_written': {'set': {'str3:0x0E': '……比克\\n　　死掉了……。'}}}),
    # ---- pages whose English shows no letter (a portrait alone, or `…`), the first of their message: scenes started
    # on the gravity-room save as above, A every 320 frames (scenes/story_empty/cont_slow.txt; the Chinese page is
    # drawn whole at once). Scene 15 branches by its story flags: `$0173` = $10
    # takes the help card (str3:0x5D, its choice, then str2:0x0F2); with `$0174` = $01 too, there is none (str3:0x5D,
    # 0x5E, then str2:0x0F1)
    'sd_empty_3C': dict(script='scenes/story_empty/cont_slow.txt', sram='grav', lo=2300, hi=2934,
                        wram=['2304:7E01D8=3C', '2304:7E1648=80']),
    'sd_empty_15a': dict(script='scenes/story_empty/cont_slow.txt', sram='grav', lo=2300, hi=3570,
                         wram=['2304:7E01D8=15', '2304:7E1648=80', '2304:7E0173=10']),
    'sd_empty_15b': dict(script='scenes/story_empty/cont_slow.txt', sram='grav', lo=2300, hi=3570,
                         wram=['2304:7E01D8=15', '2304:7E1648=80', '2304:7E0173=10', '2304:7E0174=01']),
    # ---- map menus
    'menus': dict(script='menus', lo=3300, hi=6878),
    'mm_drop': dict(script='scenes/map_menus/u_drop2.txt', lo=3400, hi=4684,
                    wram=['3680:7E0100=02C000C0', '3680:7E01B2=02'],
                    controls={'no_help2': {'drop': ['help:2']}, 'no_help02': {'drop': ['help:0', 'help:2']}}),
    'mm_aid': dict(script='scenes/map_menus/item_browse.txt', lo=3400, hi=4742,
                   wram=['3680:7E0100=00C020C021C022C023C024C025C026C027C01FC01EC0', '3680:7E01B2=0B'],
                   controls={'unit4_as_item30': {'set': {'unit:0x04': '栽培人'}},
                             'no_help31': {'drop': ['help:31']}}),
    'mm_browse1': dict(script='scenes/map_menus/item_browse.txt', lo=3400, hi=4720,
                       wram=['3680:7E0100=00C001C002C003C004C005C006C007C008C009C00AC00BC00CC00DC00EC00FC010C011C0'
                             '12C013C014C015C016C017C0', '3680:7E01B2=18']),
    'mm_browse2': dict(script='scenes/map_menus/item_browse.txt', lo=3400, hi=4720,
                       wram=['3680:7E0100=18C019C01AC01BC01CC01DC01EC01FC020C021C022C023C024C025C026C027C0',
                             '3680:7E01B2=10']),
    # ---- shop and town
    'st_item1': dict(script='scenes/shop_town/item1.txt', lo=3690, hi=4280,
                     wram=SHOP + ['3700:7E0100=0080018010401744', '3700:7E01B2=04']),
    'st_buy1': dict(script='scenes/shop_town/buy1.txt', lo=3690, hi=5000, wram=SHOP),
    'st_misc1': dict(script='scenes/shop_town/misc1.txt', lo=3690, hi=4780, wram=SHOP),
    # (the training ground: str3:0x25 YES / NO, then str3:0x27 Fight / Raise KI, the cursor moved down and up on each)
    'tr_choice': dict(script='scenes/training/train1.txt', lo=3690, hi=4900, wram=TRAIN),
    # ---- minigames, special screens
    'mg_kk': dict(script='scenes/minigames/kk_yes.txt', lo=4400, hi=4900),
    'mg_kk_b': dict(script='scenes/minigames/kk_b.txt', lo=4400, hi=5500,
                    wram=['4690:7E0B01=3A', '4690:7E0B05=50', '4690:7E0C36=80']),
    'mg_grav': dict(script='scenes/minigames/grav_a.txt', sram='grav', lo=2150, hi=3300, wram=GRAV),
    # (the gravity training's choice, `str3:0x46`: the game writes the level into its third line, `GRAVITY_CHOICE`)
    'mg_grav_choice': dict(script='scenes/minigames/grav_a.txt', sram='grav', lo=4040, hi=4700, wram=GRAV),
    # ---- v51: the gravity training names whoever trains (the English pages battle:162 and battle:165 have no name
    # slot; the game writes the trainee into slot 1 before battle:162 and into slot 2 before battle:165). The controls
    # write the trainee's name out, so the page must draw the same as the control.
    # (the gravity room with Piccolo: saves/grav-piccolo.sav, `save-edit --set party=piccolo --set map=21`)
    'mg_grav_piccolo': dict(script='scenes/minigames/grav_a.txt', sram='grav-piccolo', lo=5540, hi=5700, wram=GRAV,
                            controls={'piccolo_written': {'set': {'battle:165': '比克\\n被重力重重\\n壓住了！{auto}'}}}),
    # (King Kai's gravity trial: tools/saves/secrets.sh king-kai-gravity-piccolo, map 1; king-kai-gravity-goku, map 7)
    'kk_gravity_piccolo': dict(script='replay:secret-king-kai-gravity-piccolo', lo=3240, hi=3940,
                               controls={'piccolo_written': {'set': {
                                   'battle:162': '「比克，\\n要在{num:1}回合內\\n突破重力！」',
                                   'battle:165': '比克\\n被重力重重\\n壓住了！{auto}'}}}),
    'kk_gravity_goku': dict(script='replay:secret-king-kai-gravity-goku', lo=3420, hi=3520,
                            controls={'goku_written': {'set': {'battle:162': '「悟空，\\n要在{num:1}回合內\\n突破重力！」'}}}),
    # ---- intro, title, continue
    'ti_intro': dict(script='scenes/title_intro_ending/idle.txt', lo=3600, hi=4960),
    'ti_continue': dict(script='scenes/title_intro_ending/continue.txt', sram='nail-porunga', lo=1500, hi=2700),
}


def no(rules, *frames, key=None, allow=()):
    return {'type': 'rules', 'rules': list(rules), 'frames': [list(f) for f in frames] or None, 'key': key,
            'allow': [dict(a, rules=list(a['rules'])) for a in allow]}


def allow(rules, *frames, key=None, why=''):
    return {'rules': list(rules), 'frames': [list(f) for f in frames], 'key': key, 'why': why}


def probe(id, kind, box, frames, **kw):
    return {'type': 'probe', 'spec': dict(id=id, kind=kind, box=list(box), frames=list(frames), **kw)}


def sc(name, run, kind, area, what, checks, severity=''):
    return dict(name=name, run=run, kind=kind, area=area, what=what, checks=checks, severity=severity)


# The wish list drawn 4 frames before its window (scenario bi_F12_wishlist_before_window) happened in every
# Porunga run; the porunga_* scenarios do not count it (bi_F12 does, on porunga_13)
F12_ALLOW = allow(WRONG, (4855, 4870), key='str7:0x00', why='the wish list drawn before its window')

# The name-slot guards (sd_guard_names_*): a typed page whose English starts with a name slot types the name first,
# whose letters the scan's English of the page (text-dump) does not have, so WRONGPAGE flags those frames though the
# Chinese types as it should (checked on the pictures); and the blinking continue mark is MASKED under the paper.
def name_typing(first, last):
    return allow(['WRONGPAGE'], (first, last), key=r'str3:0x67', why='the slot name typed first (checked on the pictures)')


def arrow_f4(first, last, key):
    return allow(['MASKED'], (first, last), key=key, why='the continue mark under the paper')


# Pages whose English shows no letter (sd_empty_*): the original's blinking `▼` in the page's first cell is under the
# paper (the Chinese page is whole at once, then its `▼` goes after it), which MASKED flags every 8 frames.
def arrow_empty(first, last, key):
    return allow(['MASKED'], (first, last), key=key, why="the original's continue mark under the paper")
ARROW = [255, 82, 0]       # the original's blinking continue mark (and the window corners: boxes stay inside)

SCENARIOS = [
    # ======================================================== Porunga
    sc('porunga_13_hooks', 'porunga_13', 'fixed', 'porunga',
       '波路亞 1→3（裝 hook）：英文殘字 0 幀、中文畫錯視窗 0 幀',
       [no(ENGLISH + WRONG, allow=[F12_ALLOW])]),
    sc('porunga_13_nohooks', 'porunga_13_nohooks', 'fixed', 'porunga',
       '波路亞 1→3（不裝 hook）：英文殘字 0 幀、中文畫錯視窗 0 幀',
       [no(ENGLISH + WRONG, allow=[F12_ALLOW])]),
    sc('porunga_31', 'porunga_31', 'fixed', 'porunga',
       '波路亞 3→1：英文殘字 0 幀、中文畫錯視窗 0 幀',
       [no(ENGLISH + WRONG, allow=[F12_ALLOW])]),
    sc('porunga_2', 'porunga_2', 'fixed', 'porunga',
       '波路亞只許 2：英文殘字 0 幀、中文畫錯視窗 0 幀（願望 2 選人窗、願望清單早畫的幀由它們自己的場景管）',
       [no(ENGLISH + WRONG, allow=[F12_ALLOW])]),
    sc('porunga_d_13', 'porunga_13', 'fixed', 'porunga',
       '許完願後，許願視窗被畫成神的說明 help:0 加英文殘字（第 1、第 3 個願之後各一次）',
       [no(['WRONGPAGE'], (5690, 5720), (6575, 6610), key='help:0'), no(['REVERT'], (5690, 5720), (6575, 6610))],
       severity='high'),
    sc('porunga_d_13_nohooks', 'porunga_13_nohooks', 'fixed', 'porunga',
       '許完願後的許願視窗，同上，不裝 hook',
       [no(['WRONGPAGE'], (5690, 5720), (6575, 6610), key='help:0'), no(['REVERT'], (5690, 5720), (6575, 6610))],
       severity='high'),
    sc('porunga_d_31', 'porunga_31', 'fixed', 'porunga',
       '許完願後的許願視窗，同上，3→1',
       [no(['WRONGPAGE'], key='help:0'), no(['REVERT'])],
       severity='high'),

    # ======================================================== battle items
    sc('bi_F01_grandpa_enemy_name', 'bi_grandpa', 'fixed', 'battle_items',
       '爺爺的選敵人窗：掃描時敵人名字被畫成說明 help:7 的中文碎片，碎片在更早的修正已消失；剩下的是被蓋住的說明殘字'
       '「Pa／en／at」塗底色時，把名字欄左緣（2x x=341）選敵人窗框線的淡邊塗成紙色（已修好）',
       [probe('F01_name', 'equal', (340, 360, 500, 390), (4885, 4995), a='zh', b='no_help7')],
       severity='high'),
    sc('bi_F02_yajirobe_enemy_name', 'bi_yajirobe', 'fixed', 'battle_items',
       '亞奇洛貝的選敵人窗：掃描時敵人名字變成說明 help:18 的中文碎片，碎片在更早的修正已消失；剩下的是被蓋住的說明殘字'
       '「Ca／to」塗底色時，把名字欄左緣（2x x=341）選敵人窗框線的淡邊塗成紙色（已修好）',
       [probe('F02_name', 'equal', (340, 360, 500, 390), (4885, 4995), a='zh', b='no_help18')],
       severity='high'),
    sc('bi_F03_piccolo_enemy_name', 'bi_piccolo', 'fixed', 'battle_items',
       '比克物件的選敵人窗：敵人名字變成中文碎片（help:29）',
       [probe('F03_name', 'equal', (340, 360, 500, 390), (4890, 5095), a='zh', b='no_help29')],
       severity='high'),
    sc('bi_F04_drop_confirm', 'bi_drop_yes', 'fixed', 'battle_items',
       '丟棄確認窗：金錢框變空白、「丟掉嗎？」變碎片',
       [probe('F04_zeni', 'stable', (352, 286, 524, 322), (4928, 5050), a='zh', ref=4927),
        probe('F04_question', 'equal', (400, 360, 590, 390), (4928, 5050), a='zh', b='no_help1')],
       severity='medium'),
    sc('bi_F05_after_drop_help_over_yesno', 'bi_drop_porunga', 'fixed', 'battle_items',
       '丟棄選「是」之後，下一個道具的說明壓到還在畫面上的是／否窗',
       [no(['WRONGPAGE'], (5050, 5070))],
       severity='low'),
    sc('bi_F06_obj27_three_lines', 'bi_obj27', 'fixed', 'battle_items',
       '打噴嚏的拉琪（物件 27）訊息：3 行中文塞進 2 行的窗，壓到 KI/BP 標題列與下方視窗',
       [no(['WRONGPAGE'], (4900, 5250), key=r'str7:0x0[12]')],
       severity='medium'),
    sc('bi_help31_three_lines', 'bi_help31', 'guard', 'battle_items',
       '戰鬥中道具說明 help:31（物件 1F）的三行中文在說明框內：金錢窗與說明框下框線以下和拿掉 help:31 時一樣，'
       '第三行有字',
       [no(['WRONGPAGE'], key=r'help:3[0-2]'),
        probe('bi_h31_zeni', 'equal', (352, 286, 524, 322), (4790, 4900), a='zh', b='no_help31'),
        probe('bi_h31_below', 'equal', (276, 453, 596, 478), (4790, 4900), a='zh', b='no_help31'),
        probe('bi_h31_row3', 'ink', (300, 420, 520, 453), (4790, 4900), a='zh', dark=120, min=40)]),
    sc('bi_F07_help30_overflow', 'bi_help30', 'fixed', 'battle_items',
       'help:30（英文只有 1 行）的中文第 1 行跑到說明框上面、壓在金錢框上',
       [no(['WRONGPAGE'], key=r'help:3[0-2]')],
       severity='medium'),
    sc('bi_F08_wish2_member_pick', 'porunga_2', 'fixed', 'battle_items',
       '願望 2 的選人窗：名字變碎片、HP/KI/Lv 標籤與數字被擦掉',
       [no(['REVERT'], (5180, 5274)),
        probe('F08_members', 'equal', (156, 40, 430, 120), (5190, 5270), a='zh', b='no_wishlist'),
        probe('F08_detail', 'equal', (440, 40, 530, 214), (5190, 5270), a='zh', b='no_wishlist')],
       severity='high'),
    sc('bi_F09_after_wish2_help0_on_hpwin', 'porunga_2', 'fixed', 'battle_items',
       '願望 2 選完人之後：神的說明 help:0 畫在 HP 選人窗，接著願望清單蓋在 HP 窗上 4 幀',
       [no(['WRONGPAGE'], (5270, 5300)), no(['REVERT'], (5275, 5300))],
       severity='medium'),
    sc('bi_F10_wish1_B_help0', 'bi_p1b_long', 'fixed', 'battle_items',
       '許一個願後按 B：願望窗與道具清單窗被畫成神的說明 help:0，持續到下一次操作',
       [no(['WRONGPAGE'], (5860, 6280), key='help:0')],
       severity='high'),
    sc('bi_F11_hpwin_corrupt_after_B', 'bi_p1b_long', 'fixed', 'battle_items',
       '接上一個場景：回到戰鬥指令窗後，上方 HP 窗的名字與 HP 被擦掉，直到重新打開道具選單',
       [probe('F11_hpwin', 'stable', (160, 54, 340, 116), (6280, 7060), a='zh', ref=4740)],
       severity='high'),
    sc('bi_F12_wishlist_before_window', 'porunga_13', 'fixed', 'battle_items',
       '願望清單 str7:0x00 在願望窗出現前 4 幀，先畫在道具清單／狀態窗上',
       [no(WRONG, (4855, 4870), key='str7:0x00')],
       severity='medium'),
    sc('bi_guard_kami', 'bi_kami', 'guard', 'battle_items',
       '戰鬥中用神（選隊員、恢復）：目前乾淨',
       [no(CLEAN, allow=[allow(['UNDRAWN'], (4769, 4770), key='battle:47',
                               why='bm_F4 那則舊暫存器的 battle:47（C0）：英文不在畫面上，改看畫面上的英文後不畫；'
                                   'UNDRAWN 是因為狀態框的「Piccolo」對上它的名字插槽（看圖確認）')])]),

    # ======================================================== battle commands, cards, specials
    sc('bm_F1_prepare_over_cards', 'battle', 'fixed', 'battle_menus',
       '手動選卡按 A 後，舊的「敵人出現了！」(battle:89) 連底紙蓋在手牌上 8 幀',
       [probe('bmF1_cards', 'equal', (245, 432, 515, 484), (4755, 4768), a='zh', b='no_battle89')],
       severity='medium'),
    sc('bm_F2_runfail_paper_early', 'bm_run1', 'fixed', 'battle_menus',
       '第二次「不行，逃不掉。」窗打開前 1 幀，空白底紙先蓋在手牌上',
       [no(['ZHONLY'], (4850, 4854))],
       severity='low'),
    sc('bm_F3_order_iccolo', 'bm_menu1', 'fixed', 'battle_menus',
       '戰鬥中選單→排序關閉時，右上窗露出英文殘名 iccolo、oku',
       [no(ENGLISH, (5490, 5535))],
       severity='medium'),
    sc('bm_F4_itemcmd_battle47', 'bm_itemcmd', 'fixed', 'battle_menus',
       '第一次按「卡牌」時，錯頁「的　定身術！」(battle:47) 閃在手牌和指令窗旁 2 幀',
       [no(['ZHONLY'], (4767, 4772))],
       severity='low'),
    sc('bm_F5_item_reopen_help_early', 'bm_itemcmd', 'fixed', 'battle_menus',
       '道具清單 B 返回後再按「卡牌」，說明中文比清單早 6 幀畫在手牌上',
       [no(['ZHONLY'], (4915, 4924))],
       severity='low'),
    sc('bm_F6_kiuse_english', 'bm_sp1', 'fixed', 'battle_menus',
       '必殺技清單底部「KI-USE」的 USE 沒有換成「使用」（譯文表有 string:20）',
       [probe('bmF6_use', 'differ', (418, 424, 474, 450), (4960, 5100), a='zh', b='orig', tol=0.02)],
       severity='medium'),
    # (the translation's right word starts on the mark's cell, 2x x 404: `消` inks x 405–422 and `耗` 425–443 (frame
    # 4960), inside the mark's, U's, S's and E's cells, 404–467. Kept on the mark's right, from the U's cell, 420, `耗`
    # inks x 441–458 and the dash stays at 405–414 (measured with the mark not taken, frame 4960, dark under 200): the
    # box, x 446–459, is blank now, and with the mark not taken this scenario FAILs, 141 bad frames 4960–5100. It started at 442 for `使用`, whose `用` ended left of it;
    # `耗` reaches 443)
    sc('bm_kiuse_dash', 'bm_sp1', 'fixed', 'battle_menus',
       '必殺技清單底部「KI-USE」：右半有譯文時，中間連字號那格塗成底色、中文從那格開始，成為「KI消耗」，'
       '與 HD 能力畫面的「ＫＩ消耗」一致（修前連字號留著）',
       [probe('bm_kiuse_dash', 'ink', (446, 425, 460, 452), (4960, 5100), a='zh', max=0)],
       severity='low'),
    sc('bm_F7_levelup_flicker', 'bm_lvlup', 'fixed', 'battle_menus',
       '連續兩人升級時，第二則「比克 升到」先整行出現 8 幀，清空 1 幀再重打',
       [no(['ZHONLY'], (6960, 6972))],
       severity='low'),
    sc('bm_guard_special_needki', 'bm_sp_needki', 'guard', 'battle_menus',
       '戰鬥→手動→選卡→必殺技（KI 不足的訊息）：目前乾淨',
       [no(CLEAN)]),

    # ======================================================== story battles
    sc('bs_F1_last_message_over_cards', 'bs_ssjf', 'fixed', 'battle_story',
       '叫出手動選卡時，上一則戰鬥訊息 battle:3 連底紙在卡片列上多畫 9 幀',
       [no(['ZHONLY'], (8779, 8791))],
       severity='medium'),
    sc('bs_F2_attack_all_early', 'bs_ssjf', 'fixed', 'battle_story',
       '全體攻擊的說明 str7:0x06 在說明窗打開前 11 幀，先畫在卡片上',
       [no(['ZHONLY'], (8959, 8972))],
       severity='medium'),
    sc('bs_F3_noitem_paper_early', 'bs_ssjf_run', 'fixed', 'battle_story',
       '劇情戰沒有道具時第二次按「卡牌」：說明窗打開前，卡片上先閃 4 幀空白底紙',
       [no(['ZHONLY'], (9111, 9118))],
       severity='low'),
    # The box's right edge was 520: the translation's second line of battle:307 fills the window (132 of its 136
    # pixels), so the `▼` drawn after its last character, as the original places it (the cell after the last one),
    # is at x 510-523 of the 2x picture and the box cut it in half (zh varied 5 < 8 though it blinks with the
    # original's frame for frame). 524 is the first column of the window's right frame (its inner orange line): the
    # box takes the whole `▼` and none of the frame. Checked that the box still fails on a build without the `▼`
    sc('bs_F4_continue_arrow_battle307', 'bs_forced', 'fixed', 'battle_story',
       '等玩家按 A 時沒有閃爍的 ▼（battle:307；所有對話頁都一樣）（修法：▼ 畫在中文最後一字之後）',
       [probe('bsF4_arrow', 'blink', (226, 362, 524, 470), (2795, 3083), a='zh', color=ARROW, k=8)],
       severity='medium'),
    sc('bs_F5_pageturn_blank', 'bs_jeice', 'fixed', 'battle_story',
       '劇情對話換頁時，繁中窗口比原版早 6 幀清成空白',
       [no(['WRONGPAGE'], (2328, 2337), key=r'str2:0x0FF/p2')],
       severity='low'),
    # (the original's `▼` after `/p1` stays until the game clears the window, 2335; the engine names `/p2` from 2330)
    sc('bs_continue_arrow_pageturn', 'bs_jeice', 'fixed', 'battle_story',
       '換頁時繁中的 ▼ 比原版早消失：引擎已點名下一頁、畫面還是上一頁的那幾幀（2330–2335）沒有 ▼'
       '（修法：留著的頁記住最後被點名時在等 A，▼ 畫到原版的 ▼ 消失）',
       [probe('bs_arrow_turn', 'mark', (448, 408, 474, 430), (2322, 2340), a='zh', color=ARROW,
              obox=(384, 348, 410, 370), min=4)],
       severity='low'),
    sc('bs_guard_ssjf_cmd_menu', 'bs_ssjf_menu', 'guard', 'battle_story',
       '劇情戰（飛里沙）指令窗與選單：目前乾淨',
       [no(CLEAN)]),

    # ======================================================== map dialogue, story pages
    sc('sd_name_slot_blank', 'sd_fusion', 'fixed', 'story_dialog',
       '譯文的名字插槽 {name:1} 畫成空白：「請跟＿融合吧」「「跟＿融合的話」',
       [probe('sd_slot_165', 'ink', (312, 390, 420, 412), (2719, 2719), a='zh', dark=120, min=40),
        probe('sd_slot_166', 'ink', (312, 328, 420, 356), (2741, 2743), a='zh', dark=120, min=40)],
       severity='medium'),
    sc('sd_guard_names_s51_nail', 'sd_names_s51', 'guard', 'story_dialog',
       'str3:0x67 場景 51（尼爾一人，插槽 2 為 0）：{name:1}{name:2} 畫成「和尼爾」，和直接寫出名字的一樣，不留「以及」'
       '（日版查證；構造：grav 存檔寫 scene_id）',
       [no(CLEAN, allow=[name_typing(2550, 2568)]),
        probe('sd_names_67_nail', 'equal', (160, 350, 590, 460), (2576, 2600), a='zh', b='nail_written')]),
    sc('sd_guard_names_s2D_cargo', 'sd_names_s2D', 'guard', 'story_dialog',
       'str3:0x67 場景 2D（插槽＝丹丹、單位 0x54）：畫成「和丹丹　卡爾克」，不是英文版共用字串的「茲姆利」'
       '（日版查證；構造）',
       [no(CLEAN, allow=[name_typing(3550, 3566)]),
        probe('sd_names_67_cargo', 'equal', (160, 350, 590, 460), (3590, 3605), a='zh', b='dende_cargo_written')]),
    sc('sd_guard_names_s1E_points', 'sd_names_s1E', 'guard', 'story_dialog',
       'str3:0x10 場景 1E（插槽是第五張名字表 0 號，上游不認）：「得到了點數。」的「點數」有畫出來（改前是「得到了。」）'
       '（日版查證；構造）',
       [no(CLEAN, allow=[arrow_f4(2462, 2490, r'str3:0x10')]),
        probe('sd_names_10_points', 'ink', (270, 365, 330, 410), (2462, 2484), a='zh', dark=120, min=150)]),
    sc('sd_guard_names_s47_dead', 'sd_names_s47', 'guard', 'story_dialog',
       'str3:0x0E 場景 47（陷阱，效果 $40 把倒下的成員寫進插槽）：畫成「……比克死掉了……。」'
       '（日版查證；構造：WRAM 寫入第二筆隊伍紀錄）',
       [no(CLEAN, allow=[arrow_f4(2550, 2570, r'str3:0x0E')]),
        probe('sd_names_0E_dead', 'equal', (160, 350, 590, 460), (2556, 2564), a='zh', b='piccolo_written')]),
    # Pages whose English shows no letter: the first page of their message drew nothing (no window seen before to
    # place it). Whole from the first frame the window shows it; the
    # `▼` after the Chinese with the original's (`mark`: where the original shows its `▼`, the page's first cell, the Chinese shows it after its last character).
    sc('sd_empty_3C_113_163', 'sd_empty_3C', 'fixed', 'story_dialog',
       '場景 3C：str2:0x113、0x163（英文只有頭像，訊息的第一頁）沒畫中文；修後整頁一次出現（原為逐字打）、▼ 跟著原版出現在中文後面',
       [no(ENGLISH + WRONG, allow=[arrow_empty(2340, 2934, r'str2:0x1[16]3')]),
        probe('sd_empty_113', 'ink', (248, 300, 580, 466), (2520, 2604), a='zh', dark=120, min=1500),
        # (whole from the first frame the window shows it, 2343: lines 2 to 4 measured 1987 dark pixels; typed a
        # character every 4 frames, as before the change, they had none until about 2380)
        probe('sd_empty_113_whole', 'ink', (248, 350, 580, 466), (2343, 2380), a='zh', dark=120, min=1900),
        probe('sd_empty_113_arrow', 'mark', (496, 424, 522, 446), (2520, 2604), a='zh', color=ARROW,
              obox=(256, 316, 282, 338), min=4),
        probe('sd_empty_163', 'ink', (248, 300, 580, 466), (2750, 2924), a='zh', dark=120, min=1000),
        probe('sd_empty_163_arrow', 'mark', (496, 408, 522, 430), (2750, 2924), a='zh', color=ARROW,
              obox=(256, 316, 282, 338), min=4)],
       severity='high'),
    sc('sd_empty_15a_5D_0F2', 'sd_empty_15a', 'fixed', 'story_dialog',
       '場景 15（有援助卡）：str3:0x5D（英文只有「…」）與 str2:0x0F2（只有頭像）沒畫中文',
       [no(ENGLISH + WRONG, allow=[arrow_empty(2330, 3570, r'str3:0x5D|str2:0x0F2')]),
        probe('sd_empty_5D', 'ink', (176, 348, 584, 452), (2430, 2604), a='zh', dark=120, min=1000),
        probe('sd_empty_5D_arrow', 'mark', (488, 436, 514, 452), (2430, 2604), a='zh', color=ARROW,
              obox=(192, 364, 218, 386), min=4),
        probe('sd_empty_0F2', 'ink', (248, 300, 580, 466), (3400, 3564), a='zh', dark=120, min=1000)],
       severity='high'),
    # (the choice after `str3:0x5D`: the game draws its cursor alone, no word, in the page's second line; the Chinese
    # 是 / 否 come with the page, the cursor keeps its picture)
    sc('sd_empty_15a_5D_cursor', 'sd_empty_15a', 'fixed', 'story_dialog',
       '場景 15：str3:0x5D 之後的選項，遊戲只畫游標 ▶：中文整頁照畫（是／否），游標不被底紙蓋掉',
       [probe('sd_empty_5D_cursor', 'equal', (436, 398, 452, 414), (2620, 2924), a='zh', b='orig'),
        probe('sd_empty_5D_choice', 'ink', (176, 348, 584, 452), (2620, 2924), a='zh', dark=120, min=1000)],
       severity='high'),
    # (the choice's place: the cursor `▶` at 2x x 436-449, 是 from x 470, the nearest a whole full-width space puts it
    # without the cursor's cell, column 17: 12 spaces of 12 px, 144 px for 136; one fewer would draw over the cursor)
    sc('sd_choice_5D_place', 'sd_empty_15a', 'guard', 'story_dialog',
       'str3:0x5D 的選項：「是」從游標右邊第一個全形位置開始（2x x 466–478 有字），游標和「是」之間沒有字'
       '（選項前 12 個全形空白，少一個會壓到游標）',
       [probe('sd_choice_5D_gap', 'ink', (451, 396, 466, 420), (2620, 2924), a='zh', dark=120, max=0),
        probe('sd_choice_5D_yes', 'ink', (466, 396, 478, 420), (2620, 2924), a='zh', dark=120, min=10)]),
    sc('sd_choice_0EE_place', 'sd_fusion_choice', 'fixed', 'story_dialog',
       'str2:0x0EE（丹丹要求融合）的選項「融合／不融合」比游標右偏 32 像素（text/pages.txt 前面 8 個全形空白；英文在'
       '第 8 欄＝64 px）；改成 6 個（72 px，不壓到游標的最近位置）後「融」從 2x x 406 開始',
       [no(ENGLISH + WRONG),
        probe('sd_choice_0EE_gap', 'ink', (388, 412, 404, 436), (3210, 3300), a='zh', dark=120, max=0),
        probe('sd_choice_0EE_fuse', 'ink', (404, 412, 418, 436), (3210, 3300), a='zh', dark=120, min=10)],
       severity='medium'),
    sc('sd_empty_15b_5E_0F1', 'sd_empty_15b', 'fixed', 'story_dialog',
       '場景 15（沒有援助卡）：str3:0x5E（英文只有「…」）與 str2:0x0F1（只有頭像）沒畫中文；'
       '0x5E 不留到接著打開的對話窗上',
       [no(ENGLISH + WRONG, allow=[arrow_empty(2330, 3570, r'str3:0x5[DE]|str2:0x0F1')]),
        probe('sd_empty_5E', 'ink', (176, 348, 584, 452), (3070, 3244), a='zh', dark=120, min=1000),
        probe('sd_empty_5E_arrow', 'mark', (464, 424, 490, 446), (3070, 3244), a='zh', color=ARROW,
              obox=(192, 364, 218, 386), min=4),
        probe('sd_empty_5E_gone', 'equal', (176, 300, 584, 466), (3247, 3253), a='zh', b='orig'),
        probe('sd_empty_0F1', 'ink', (248, 300, 580, 466), (3300, 3564), a='zh', dark=120, min=1000)],
       severity='high'),
    sc('sd_intro_pageturn_1frame', 'battle', 'fixed', 'story_dialog',
       '開場劇情頁換頁時早 1 幀清成空白底紙',
       [no(['WRONGPAGE'], (2600, 2960), key=r'str3:.*')],
       severity='low'),
    sc('sd_black_screen_paper', 'sd_porunga', 'fixed', 'story_dialog',
       '畫面切成全黑時，中文對話框的底紙和字留在黑畫面上 1 幀',
       [no(['MASKED', 'ZHONLY'], (10097, 10101))],
       severity='low'),
    sc('sd_continue_arrow_missing', 'battle', 'fixed', 'story_dialog',
       '中文對話框沒有「按 A 繼續」的閃爍 ▼（開場旁白頁）（修法：▼ 畫在中文最後一字之後）',
       [probe('sd_arrow', 'blink', (160, 346, 588, 456), (2645, 2680), a='zh', color=ARROW, k=8)],
       severity='medium'),
    sc('sd_continue_arrow_pageturn', 'battle', 'fixed', 'story_dialog',
       '開場旁白換頁時繁中的 ▼ 比原版早 1 幀消失（2674：引擎已點名下一頁、畫面還是上一頁）',
       [probe('sd_arrow_turn', 'mark', (392, 408, 418, 430), (2660, 2682), a='zh', color=ARROW,
              obox=(400, 396, 426, 418), min=4)],
       severity='low'),
    # (the `▼` cell after an English line that ran past the window: the original shows the screen or a portrait there
    # until the engine writes the `▼`, 9590 / 10188–10189 / 10214–10215, and a `▼` after it, 9591–9595 / 10216–10219)
    sc('sd_continue_arrow_false', 'sd_porunga_arrow', 'fixed', 'story_dialog',
       '假 ▼：英文行伸出視窗時，繁中把那一格的深藍底或人像一角當成 ▼ 抄到中文後面'
       '（修法：只認 tile-map 上真的 ▼ 圖塊，畫面取覆蓋層動筆前的）',
       [probe('sd_false_9590', 'mark', (526, 316, 554, 344), (9586, 9595), a='zh', color=ARROW,
              obox=(608, 124, 634, 146), min=4, paper=[255, 239, 206]),
        probe('sd_false_10188', 'mark', (382, 420, 410, 448), (10184, 10189), a='zh', color=ARROW,
              obox=(532, 380, 550, 400), min=4, paper=[255, 239, 206]),
        # (battle:273: the box follows the translation's last character, 「跟我走。」 now, a character shorter than the
        # earlier 「給我過來。」: 24 pixels left of where it was, the same size)
        probe('sd_false_10214', 'mark', (456, 440, 482, 462), (10210, 10219), a='zh', color=ARROW,
              obox=(530, 412, 554, 434), min=4, paper=[255, 239, 206])],
       severity='low'),
    # (`str4:0x03`'s `You have found` / `Dragon Balls.` ran past the wish window: `und` at 1x x 290-313 y 31, `s.` and
    # the `▼` at x 290-313 y 63, until 9914 on the original; the overlay starts at 9601, after that page. From 9608,
    # the next page's first cell: on the frames before it, its window cleared, the overlay started cold does not know
    # yet where the window is)
    sc('sd_wish_spill_cold', 'sd_porunga_cold', 'fixed', 'story_dialog',
       '讀檔起跑（覆蓋層沒看過上一頁）：波路亞許願視窗右外側留著上一頁的英文 und、s.▼'
       '（修法：表 4 的頁也直接辨認視窗外的字母，連同後面的 ▼）',
       [no(ENGLISH + WRONG),
        probe('sd_wish_und', 'differ', (580, 62, 628, 78), (9608, 9700), a='zh', b='orig', tol=0.5),
        probe('sd_wish_s_dot', 'differ', (580, 126, 612, 142), (9608, 9700), a='zh', b='orig', tol=0.5),
        probe('sd_wish_arrow', 'differ', (612, 126, 628, 142), (9608, 9700), a='zh', b='orig', tol=0.5),
        # (the portrait's window left of the wish window, its frame at 2x x 166-168 and 364-366: left as it is)
        probe('sd_wish_portrait', 'equal', (160, 28, 370, 446), (9608, 9700), a='zh', b='orig')],
       severity='low'),
    # (Vegeta's page whose last line ran past the dialogue window to his portrait: the engine never wrote its `▼`, A
    # turned the page first; the cell after the line, 1x (266, 191), keeps the portrait as the original shows it)
    sc('sd_spill_mark_portrait', 'sd_porunga_arrow', 'fixed', 'story_dialog',
       '英文行伸出視窗、▼ 還沒寫就換頁時，繁中把頭像上那一格（達爾的耳朵）換成上方那格的圖（黑底加一條黑線）'
       '（修法：那一格要 tile-map 上真的有 ▼ 圖塊才算殘字）',
       [probe('sd_spill_ear', 'equal', (530, 380, 552, 400), (10190, 10225), a='zh', b='orig')],
       severity='medium'),
    sc('sd_guard_zfighters', 'sd_zfighters', 'guard', 'story_dialog',
       '那美克星 Z 戰士劇情對話：目前乾淨',
       [no(CLEAN)]),
    sc('st_dayslater_cold', 'st_namek_trip_cold', 'fixed', 'story_dialog',
       '讀檔起跑（覆蓋層沒看過這一頁被點名）：前往那美克星的「34 days later…」整頁英文，約 450 幀'
       '（過場在這頁之後停住訊息、引擎不點名；修法：沒有訊息、位元組停在某頁開頭時，點名前一頁）。'
       '讀檔後第一幀（4901）不算：那一幀的起始頁是讀檔前一幀的點名，shots 沒有記到，每一頁都一樣',
       # (the probe's first frame, 4902, was set after a run showed 4901 failing: a run started from a state draws no
       # page on its first frame, the page the frame before named not noted. A cold probe starting past the state's
       # first frames has a precedent, `sd_wish_spill_cold`: 9608 for its state at 9600, there for 7 frames whose page
       # the overlay cannot place yet)
       [no(ENGLISH + WRONG),
        probe('st_dayslater_line', 'differ', (176, 362, 412, 386), (4902, 5336), a='zh', b='orig', tol=0.05)],
       severity='medium'),
    sc('st_guard_namek_trip', 'st_namek_trip', 'guard', 'story_dialog',
       '前往那美克星的字幕（str3:0x4B～0x4C/p2）逐幀：目前乾淨，「34 days later…」那頁一直是中文',
       [no(CLEAN),
        probe('st_trip_line', 'differ', (176, 362, 412, 386), (4850, 5336), a='zh', b='orig', tol=0.05)]),

    # ======================================================== map menus
    sc('mm_save_slot_es_o', 'menus', 'fixed', 'map_menus',
       '存檔：「存檔到幾號？」窗蓋上「是/否」窗時，右邊露出英文 es、o',
       [no(ENGLISH, (6470, 6585))],
       severity='medium'),
    sc('mm_order_iccolo', 'menus', 'fixed', 'map_menus',
       '排序：右邊順位窗裡被原版切掉首字的名字（iccolo、oku）露出英文',
       [no(ENGLISH, (5475, 5550))],
       severity='medium'),
    sc('mm_guard_menus', 'menus', 'guard', 'map_menus',
       '地圖選單（能力、排序前段、速度、說明、卡牌）：目前乾淨（存檔、排序的已知問題除外）',
       [no(CLEAN)]),
    sc('mm_drop_confirm', 'mm_drop', 'fixed', 'map_menus',
       '丟棄確認「丟掉嗎？」時金錢窗被清空、問句缺字',
       [probe('mm_zeni', 'stable', (436, 286, 604, 322), (3931, 3995), a='zh', ref=3925),
        probe('mm_question', 'equal', (484, 360, 640, 390), (3936, 3995), a='zh', b='no_help2')],
       severity='low'),
    sc('mm_drop_help_over_question', 'mm_drop', 'fixed', 'map_menus',
       '丟棄確認窗打開時，說明 help:2／help:0 的中文畫在「Drop item?」窗上蓋住問句',
       [probe('mm_q_open', 'equal', (484, 360, 640, 390), (3998, 4006), a='zh', b='no_help02'),
        probe('mm_q_again', 'equal', (484, 360, 640, 390), (4248, 4259), a='zh', b='no_help02')],
       severity='low',),
    sc('mm_help30_32_overflow', 'mm_aid', 'fixed', 'map_menus',
       '道具說明 help:30/31/32 的中文多行溢出，第一行畫到金錢窗上',
       [no(['WRONGPAGE'], key=r'help:3[0-2]')],
       severity='medium'),
    # (help:31's three lines in the help box: its paper 2x y 345-452 as in the English, the third line's ink to 450;
    # the box's lower frame from 453 and the money window above as without the page; frames help:31 is up)
    sc('mm_help31_three_lines', 'mm_aid', 'guard', 'map_menus',
       '道具說明 help:31 的三行中文在說明框內：金錢窗與說明框下框線以下和拿掉 help:31 時一樣，第三行有字',
       [check for at in ((4076, 4097), (4546, 4573)) for check in (
           probe(f'mm_h31_zeni_{at[0]}', 'equal', (436, 286, 604, 322), at, a='zh', b='no_help31'),
           probe(f'mm_h31_below_{at[0]}', 'equal', (360, 453, 680, 478), at, a='zh', b='no_help31'),
           probe(f'mm_h31_row3_{at[0]}', 'ink', (380, 420, 660, 453), at, a='zh', dark=120, min=40))]),
    sc('mm_item30_name_unit04', 'mm_aid', 'fixed', 'map_menus',
       '道具 id 30 在清單上叫「球根人」（敵人 unit:0x04 的譯名），譯文表 object:30 是「栽培人」',
       [probe('mm_item30', 'equal', (262, 120, 336, 148), (4110, 4150), a='zh', b='unit4_as_item30')],
       severity='low'),
    sc('mm_help_early_6f', 'mm_browse1', 'fixed', 'map_menus',
       '道具清單移動游標時，中文說明比原版早約 6 幀換成下一個道具',
       [no(['WRONGPAGE'], key=r'help:.*')],
       severity='low'),
    sc('mm_aid_label_revert', 'mm_browse2', 'fixed', 'map_menus',
       '清單有道具 id 27 時，INT/EXT/AID 標籤窗的「其他」退回英文 AID',
       [no(ENGLISH, (4040, 4660))],
       severity='low'),

    # ======================================================== shop and town
    sc('st_item_open_wrongpage', 'st_item1', 'fixed', 'shop_town',
       '城鎮指令窗選「卡牌」時，別頁的中文（battle:116）蓋在指令窗上 2 幀',
       [no(['WRONGPAGE'], (3905, 3912))],
       severity='low'),
    sc('st_item_reopen_stale_help', 'st_item1', 'fixed', 'shop_town',
       '第二次開卡牌畫面時，上一張卡的說明在說明窗出現前浮在地圖上 2 幀',
       [no(['ZHONLY'], (4115, 4120))],
       severity='low'),
    sc('st_item_help_early', 'st_item1', 'fixed', 'shop_town',
       '城鎮卡牌畫面移動游標時，中文說明比原版早 6 幀換成下一張卡',
       [no(['WRONGPAGE'], (3985, 4045), key=r'help:.*')],
       severity='low'),
    sc('st_shop_arrow_missing', 'st_buy1', 'fixed', 'shop_town',
       '商店訊息等玩家按 A 時，原版的閃爍 ▼ 在繁中版被底紙蓋掉（修法：▼ 畫在中文最後一字之後）',
       [probe('st_arrow', 'blink', (250, 300, 588, 420), (4760, 4838), a='zh', color=ARROW, k=8)],
       severity='medium'),
    sc('st_guard_misc1', 'st_misc1', 'guard', 'shop_town',
       '卜卦婆婆的店（交談、調查、選單）：目前乾淨',
       [no(CLEAN)]),
    # (the training ground's choice, str3:0x27: the game draws its cursor ▶ in column 14 (03:A3F3, LDX #$05DC), the
    # choices from column 16, a blank cell between; the overlay took only a cursor right before a letter, and painted
    # it over. The cursor's cell must show as in the original on the row chosen: 2x box of column 14, rows 23 and 25)
    sc('tr_menu_cursor', 'tr_choice', 'fixed', 'shop_town',
       '修業場選單「要選哪個？　戰鬥修業／氣勢修業」：游標 ▶（第 14 欄，和選項隔一格空白）在選到的那一列照原版顯示，'
       '不被底紙蓋掉（4460–4620 選第一列、4628–4824 選第二列）；整段沒有英文殘字',
       [no(ENGLISH + WRONG),
        probe('tr_fight_cursor', 'equal', (340, 366, 356, 382), (4460, 4620), a='zh', b='orig'),
        probe('tr_raise_cursor', 'equal', (340, 398, 356, 414), (4628, 4824), a='zh', b='orig')],
       severity='high'),
    # (the YES / NO before it, str3:0x25: the cursor of 01:D54C in column 20, right before YES and NO in column 21)
    sc('tr_yesno_cursor', 'tr_choice', 'guard', 'shop_town',
       '修業場「要修業嗎？　是／否」：游標 ▶（第 20 欄，緊貼選項）選「否」（4256–4332）與選「是」（4338–4416）時都照原版顯示',
       [probe('tr_yes_cursor', 'equal', (436, 398, 452, 414), (4338, 4416), a='zh', b='orig'),
        probe('tr_no_cursor', 'equal', (436, 430, 452, 446), (4256, 4332), a='zh', b='orig')]),

    # ======================================================== minigames, special screens
    sc('mg_battle89_flash', 'mg_kk', 'fixed', 'minigames',
       '一般戰鬥選完卡片時，「敵人出現了！」(battle:89) 連底紙蓋在手牌上約 8 幀',
       [no(['ZHONLY'], (4877, 4888))],
       severity='medium'),
    sc('mg_grav_intro_early_clear', 'mg_grav', 'fixed', 'minigames',
       '重力室開場對話換頁時，中文比原版早 6 幀清空',
       [no(['WRONGPAGE'], (2450, 2465), key=r'str2:0x11B/p2')],
       severity='low'),
    sc('mg_guard_kaioken_b', 'mg_kk_b', 'guard', 'minigames',
       '界王拳提示按 B 取消：目前乾淨',
       [no(CLEAN)]),
    sc('mg_guard_grav_choice', 'mg_grav_choice', 'guard', 'minigames',
       '重力室選倍數（str3:0x46，遊戲改寫第三行的倍數）：頁打完後上下改倍數時，中文與倍數照常（打字中那段由下一個場景管）',
       [no(CLEAN, (4247, 4700))]),
    sc('mg_grav_choice_typing', 'mg_grav_choice', 'fixed', 'minigames',
       '重力室選倍數頁打到第三行時，中文的倍數先冒出「1」（state 5 把插槽算成已寫）、之後只剩「1」而原版已是「10x」'
       '（已修好：中文的數字整個出現，4203 起是「10」）；之後還紅的 WRONGPAGE 4215–4222 是偵測規則的誤判：'
       '數字插槽的值後面接著原文正從開頭打出的列（「10x」「10xG」）被當成別頁的字，scan.py 的 row_fits 改成依覆蓋層記錄的'
       '插槽數字（nums=）認得這種列後通過。接受不修的兩個時間差：原版只有「1」的 4 幀（4203–4206）中文已是「10」；'
       '原版打出「x」（4215）之後中文的「倍」要到 4231 才出現（晚 16 幀，逐字頁依已出現英文字數的比例露出中文）',
       [no(['ZHONLY'], (4195, 4246)), no(['WRONGPAGE'], (4195, 4246), key=r'str3:0x46')],
       severity='low'),
    sc('mg_guard_grav', 'mg_grav', 'guard', 'minigames',
       '重力室開場與修業：目前乾淨（換頁早清那 6 幀除外）',
       [no(CLEAN)]),
    sc('mg_grav_piccolo_165', 'mg_grav_piccolo', 'fixed', 'minigames',
       '比克在太空船重力室受訓時，battle:165 寫「比克被重力重重壓住了！」（v1.0 寫死「悟空」；插槽 2＝0x8001）',
       [probe('grav_165_piccolo', 'equal', (160, 350, 590, 460), (5596, 5636), a='zh', b='piccolo_written')],
       severity='medium'),
    sc('kk_gravity_piccolo_names', 'kk_gravity_piccolo', 'fixed', 'minigames',
       '比克在界王星（map 1）重力試煉：battle:162「「比克，要在10回合內突破重力！」」、battle:165「比克被重力…」'
       '（v1.0 兩頁都寫死「悟空」，Ming 2026-11-23 實玩回報；插槽 1／2＝0x8001）',
       [probe('kk_162_piccolo', 'equal', (160, 350, 590, 460), (3296, 3324), a='zh', b='piccolo_written'),
        probe('kk_165_piccolo', 'equal', (160, 350, 590, 460), (3868, 3928), a='zh', b='piccolo_written')],
       severity='medium'),
    sc('kk_guard_gravity_goku_162', 'kk_gravity_goku', 'guard', 'minigames',
       '悟空在界王星（map 7）重力試煉：battle:162 照樣是「「悟空，要在10回合內突破重力！」」（插槽 1＝0x8000）',
       [probe('kk_162_goku', 'equal', (160, 350, 590, 460), (3472, 3500), a='zh', b='goku_written')]),

    # ======================================================== intro, title, continue
    sc('ti_intro_last_line_sliver', 'ti_intro', 'fixed', 'title_intro_ending',
       '開場字幕最後一行離開畫面頂端時，中文早一幀消失，頂端露出英文字底 1–2 像素',
       [no(ENGLISH, (4920, 4926))],
       severity='low'),
    sc('ti_guard_intro', 'ti_intro', 'guard', 'title_intro_ending',
       '開場字幕捲動：目前乾淨',
       [no(CLEAN)]),
    sc('ti_guard_continue', 'ti_continue', 'guard', 'title_intro_ending',
       '標題→繼續遊戲→選存檔→進地圖：目前乾淨',
       [no(CLEAN)]),
]
