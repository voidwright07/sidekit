"""Read-only validation of the delivered native files and animation container."""
import json
import struct
import zlib
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
meta = json.loads((ROOT / 'fox-ninja.json').read_text())
pixels = json.loads((ROOT / 'source/pixels.json').read_text())
palette = {tuple(c) for c in pixels['palette'].values()}
atlas = Image.open(ROOT / 'fox-ninja.png').convert('RGBA')
assert atlas.size == (128, 80)
assert len(meta['frames']) == 40
assert set(atlas.getchannel('A').getdata()) == {0, 255}
assert set(atlas.getdata()) <= palette
names = list(meta['frames'])
native = []
for name, info in meta['frames'].items():
    clip = name.rsplit('_', 1)[0]
    im = Image.open(ROOT / 'frames' / clip / (name + '.png')).convert('RGBA')
    assert im.size == (16, 16), name
    assert im.getbbox(), name
    r = info['frame']
    assert atlas.crop((r['x'], r['y'], r['x'] + 16, r['y'] + 16)).tobytes() == im.tobytes(), name
    native.append(im.tobytes())
for row, (name, frames) in enumerate(pixels['animations'].items()):
    strip = Image.open(ROOT / 'strips' / (name + '.png')).convert('RGBA')
    mirrored = Image.open(ROOT / 'strips' / (name + '_mirrored.png')).convert('RGBA')
    assert strip.size == mirrored.size == (128, 16)
    for i in range(8):
        tile = strip.crop((i * 16, 0, i * 16 + 16, 16))
        mtile = mirrored.crop((i * 16, 0, i * 16 + 16, 16))
        assert tile.tobytes() == native[row * 8 + i]
        assert tile.transpose(Image.Transpose.FLIP_LEFT_RIGHT).tobytes() == mtile.tobytes()
        expected = bytes(v for line in frames[i] for char in line for v in pixels['palette'][char])
        assert expected == tile.tobytes()
    if name == 'idle':
        assert all(max(y for y, line in enumerate(frame) if any(c != '.' for c in line)) == 14 for frame in frames)
    if name == 'run':
        assert len({tuple(frame) for frame in frames}) == 8
        assert all(max(y for y, line in enumerate(frame) if any(c != '.' for c in line)) == (12 if i in (3, 7) else 14) for i, frame in enumerate(frames))

data = (ROOT / 'fox-ninja.aseprite').read_bytes()
u16 = lambda offset: struct.unpack_from('<H', data, offset)[0]
u32 = lambda offset: struct.unpack_from('<I', data, offset)[0]
assert u32(0) == len(data)
assert (u16(4), u16(6), u16(8), u16(10), u16(12)) == (0xA5E0, 40, 16, 16, 32)
pos, cel_count, tags = 128, 0, []
for index in range(40):
    end = pos + u32(pos)
    assert u16(pos + 4) == 0xF1FA
    assert u16(pos + 8) == meta['frames'][names[index]]['duration']
    count = u16(pos + 6)
    pos += 16
    for _ in range(count):
        chunk_end, kind, body = pos + u32(pos), u16(pos + 4), pos + 6
        if kind == 0x2004:
            assert u16(body) & 3 == 3  # visible and editable
        elif kind == 0x2018:
            ntags, p = u16(body), body + 10
            for _ in range(ntags):
                first, last = u16(p), u16(p + 2)
                length = u16(p + 17)
                name = data[p + 19:p + 19 + length].decode()
                tags.append({'name': name, 'from': first, 'to': last, 'direction': 'forward'})
                p += 19 + length
            assert p == chunk_end
        elif kind == 0x2005:
            assert u16(body) == 0
            assert (u16(body + 2), u16(body + 4), data[body + 6], u16(body + 7)) == (0, 0, 255, 2)
            assert (u16(body + 16), u16(body + 18)) == (16, 16)
            assert zlib.decompress(data[body + 20:chunk_end]) == native[index]
            cel_count += 1
        pos = chunk_end
    assert pos == end
assert pos == len(data) and cel_count == 40
assert tags == meta['meta']['frameTags']

gif_counts = {}
for file in (ROOT / 'previews').glob('*.gif'):
    gif = Image.open(file)
    frames = gif.n_frames
    total = 0
    for i in range(frames):
        gif.seek(i)
        gif.load()
        total += gif.info['duration']
        if file.stem in pixels['animations']:
            # Each eight-fold enlarged GIF frame must exactly match the PNG
            # on the documented opaque preview background.
            image = gif.convert('RGBA')
            for y, line in enumerate(pixels['animations'][file.stem][i]):
                for x, char in enumerate(line):
                    color = pixels['palette'][char] if char != '.' else [24, 28, 43, 255]
                    assert image.getpixel((x * 8 + 4, y * 8 + 4)) == tuple(color)
    assert frames == (120 if file.stem == 'animations' else 8)
    if file.stem == 'animations':
        assert total == 2400
    gif_counts[file.name] = {'frames': frames, 'duration_ms': total, 'bytes': file.stat().st_size}

print(json.dumps({'status': 'PASS', 'native_frames': 40, 'frame_size': [16, 16], 'atlas_size': list(atlas.size), 'opaque_colors_used': len(set(atlas.getdata())) - 1, 'alpha_values': [0, 255], 'aseprite_cels_verified': cel_count, 'animation_tags_verified': len(tags), 'gif_previews': gif_counts}, indent=2))
