# Fox Ninja — native 16×16

An original orange fox with cream muzzle and tail tip, indigo ninja clothes, and a red headband. Five animations, eight timeline frames each. Final sprites are authored directly as 16×16 pixel arrays, with hard edges and binary transparency.

## Open first

- `preview.html`: offline animated viewer, with pause, speed, mirror, pixel grid, and light background controls. Includes actual-size 1× previews.
- `fox-ninja.png`: transparent **128×80** atlas. Slice into **16×16**, 8 columns, 5 rows, no margins or spacing.
- `fox-ninja.aseprite`: editable **16×16** document, 40 cels, one editable layer, named animation tags, palette, and per-frame timings.
- `fox-ninja.json`: atlas rectangles, frame tags, per-frame timing, loop settings, and suggested import configuration.

## Animation order

| Row (zero-based) | Animation | Frames | Duration | Playback |
| --- | --- | --- | --- | --- |
| 0 | Idle | 8 | 1200 ms | Loop |
| 1 | Run | 8 | 480 ms | Loop |
| 2 | Dash | 8 | 400 ms | Once; frames 2–5 may loop during a held dash |
| 3 | Wall climb | 8 | 800 ms | Loop |
| 4 | Wall jump | 8 | 680 ms | Once |

Indices in the JSON are zero-based. Dash deliberately repeats a hold pose. The eight frames are timeline frames, not a claim that all poses in every clip are unique.

Idle includes breathing, a blink, and tail motion. Run alternates contact, compression, passing, and flight poses. Dash has anticipation, a stretched horizontal pose, and recovery. Wall climb alternates reaching and pulling hands. Wall jump compresses at the wall, extends the legs, turns away, and rises.

## Files

- `strips/`: one 128×16 transparent PNG per animation, plus mirrored versions.
- `frames/`: forty individual 16×16 transparent PNGs.
- `previews/`: GIF animations, a labelled contact sheet, and enlarged PNGs. These are viewing aids; import the native files for gameplay.
- `palette.hex`: authoring palette.
- `source/pixels.json`: editable character grids, one symbol per pixel, with RGBA palette values.
- `source/build.cjs`: reproducible integer-grid authoring and export source. Requires Node.js and `pngjs`; `sharp` is optional for smaller GIFs. Run the script to regenerate exports from its pose definitions. Editing `pixels.json` alone does not change the build source; it is an alternate exported representation.
- `source/validate.py`: read-only dimension, alpha, palette, atlas, mirrored strip, GIF, and Aseprite-cel checks; requires Pillow.
- `reference/generated-concept.png`: the built-in ImageGen concept sheet. This is a high-resolution visual reference, **not a native game asset**.
- `reference/imagegen-prompt.txt`: the complete prompt used with built-in ImageGen. The final native frames were separately authored on a 16×16 grid; the concept was not downsampled.

## Import and gameplay

Use nearest-neighbor / point filtering, no mipmaps, no lossy texture compression, and integer display scales. Suggested density: 16 pixels per unit.

Pixel coordinates use top-left origin. Ground pivot is **(8, 15)**, measured on cell edges: a pixel on row 14 meets the floor at y=15. Idle and grounded run poses share that floor; the two run flight poses lift off it. Keep the actor pivot fixed; do not trim each frame independently.

A suggested symmetric body collider is **x=5, y=3, w=6, h=12**. Treat ears, tail, and headband as decorative. Tune the collider to your game's mechanics, and keep it stable through the animation instead of deriving collision from alpha bounds.

Default idle/run/dash face right. Wall climb contacts a wall on the right. Wall jump begins against a right-side wall and faces left from frame 2. Mirror the entire corresponding strip for a left-side wall. Do not apply an extra facing flip partway through the supplied wall-jump clip.

World movement is controlled by the game. These frames do not move the actor through space. Apply dash and wall-jump velocity immediately on input; the optional anticipation pose must not delay controls. Select suitable animation entry/exit frames when states are interrupted.

## Reference and provenance

Visual inspiration: [Pixel Frog — Pixel Adventure](https://pixelfrog-assets.itch.io/pixel-adventure-1), especially compact character proportions and readable platforming poses. No source sprites from that pack were copied into these exports.

The concept was made with the built-in ImageGen tool. The final deliverable was authored directly in discrete native pixels, with a limited shared palette and alpha values of only 0 or 255. The high-resolution concept differs from the deliberately simplified native result.

The Aseprite document uses the [official Aseprite file format](https://github.com/aseprite/aseprite/blob/main/docs/ase-file-specs.md). Its decoded cels and timings are checked against the PNG exports. Aseprite itself was not launched in this session.
