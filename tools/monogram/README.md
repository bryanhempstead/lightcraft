# LightCraft profile for the Monogram Creative Console

`LightCraft.monogram` is Bryan's Monogram profile "Lightroom 1" re-assigned for LightCraft: every
slider, dial and key sends MIDI (channel 1) instead of driving Lightroom's plugin, and LightCraft
maps that MIDI to the same sliders and actions (Settings ▸ ctrl.).

| Module | Lightroom 1 | MIDI (channel 1) | LightCraft does |
|---|---|---|---|
| slider Ajn | Highlights | CC 20 | Highlights (absolute) |
| slider Ak2 | Shadows | CC 21 | Shadows (absolute) |
| slider CJ\| | Contrast | CC 22 | Contrast (absolute) |
| dial AP> | turn straighten · press+turn crop X · press O · double-tap reset | CC 30 · CC 50 · notes 60, 61 | crop angle · move crop sideways · O (crop guides) · reset angle |
| dial C7J | turn Exposure · press+turn Tint · press ⇧M | CC 31 · CC 51 · note 68 | Exposure · Tint · radial gradient |
| dial C7\` | turn Blacks · press+turn Whites · press Y | CC 32 · CC 52 · note 69 | Blacks · Whites · before/after |
| dial CEZ | turn crop scale · press+turn crop Y · press ⌘X | CC 33 · CC 53 · note 70 | zoom crop · move crop up/down · ⌘X (nothing bound in LightCraft) |
| dial C{> | press preset · press+turn Saturation · left/right turn ↓/↑ | note 71 · CC 54 | preset "BH - NMD - 7 - TD" · Saturation · (↓/↑ not mapped: no hovered-slider adjust) |
| dial H\|r | turn Temp · press+turn Lights · press W · double-tap reset Blacks | CC 34 · CC 55 · notes 72, 73 | Temp · Tone Curve Lights · WB selector · reset Blacks |
| key BFo | press preset · hold [preset HardBlack, U] | notes 62, 63 | "BH - NMD - 7 - TD" · macro: "BH - B&W - HardBlack" then unflag |
| key BH6 | press before/after · hold F | notes 64, 65 | before/after · full-screen preview |
| key BI\` | press F · hold preset | notes 66, 67 | full-screen preview · "BH - Iceland - Greens" |

Dials send relative CC (LightCraft reads both two's-complement and 64-centred encodings); one tick
moves the slider by its LrSuperKeys step (Exposure 0.05, Temp 25, most others 1).

The exact numbers are in the file and in LightCraft's `keymap.json` (`midi` list) — they were
generated together by `keys.import {"source": "monogram"}`, so they always match.

## Import it

1. Quit nothing; open **Monogram Creator**.
2. Profiles ▸ **Import Profile…** (or drag `LightCraft.monogram` onto the profile list) and pick
   this file. It appears as "LightCraft (Lightroom 1)". Your "Lightroom 1" profile is untouched.
3. If Creator doesn't switch to it by itself when LightCraft is in front, open the profile's settings
   and set its app to **LightCraft** (bundle id `ai.storyteller.lightcraft`, the .app in
   `target/release/LightCraft.app`), or pick the profile by hand.
4. In LightCraft: Settings (⌘,) ▸ **ctrl.** ▸ tick **MIDI in**. The page shows the MIDI sources it
   hears ("Monogram…") and the last message; turn a dial to check.

To change a mapping without touching Monogram: ctrl. ▸ **learn.**, move the control, pick the slider
(tick "dial" for endless dials) or the action, **add.**

Regenerate after changing the Lightroom profile: ctrl. ▸ **monogram.** (writes
`~/Library/Application Support/LightCraft/LightCraft.monogram`). LightCraft only reads Monogram's
`state.json`; it never writes inside Monogram's folders.
