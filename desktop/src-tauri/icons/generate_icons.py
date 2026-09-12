#!/usr/bin/env python3
"""Generate minimal icon files for Tauri build."""
import struct
import zlib
import os

def create_png(width, height, filepath):
    """Create a simple colored PNG."""
    # Purple color (Conduit brand)
    r, g, b, a = 99, 102, 241, 255  # #6366F1

    # Create RGBA pixel data
    raw_data = b''
    for y in range(height):
        raw_data += b'\x00'  # filter byte
        for x in range(width):
            raw_data += struct.pack('BBBB', r, g, b, a)

    # Create IHDR
    ihdr_data = struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)
    ihdr = b'IHDR' + ihdr_data
    ihdr_chunk = struct.pack('>I', len(ihdr_data)) + ihdr + struct.pack('>I', zlib.crc32(ihdr) & 0xFFFFFFFF)

    # Create IDAT
    compressed = zlib.compress(raw_data)
    idat = b'IDAT' + compressed
    idat_chunk = struct.pack('>I', len(compressed)) + idat + struct.pack('>I', zlib.crc32(idat) & 0xFFFFFFFF)

    # Create IEND
    iend = b'IEND'
    iend_chunk = struct.pack('>I', 0) + iend + struct.pack('>I', zlib.crc32(iend) & 0xFFFFFFFF)

    # Write PNG
    with open(filepath, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n')
        f.write(ihdr_chunk)
        f.write(idat_chunk)
        f.write(iend_chunk)

def create_ico(size, filepath):
    """Create a minimal ICO file."""
    # Create a simple 1x1 PNG for the ICO
    import io

    r, g, b, a = 99, 102, 241, 255

    # Create the BMP data (DIB header + pixels)
    # BMP info header (40 bytes)
    bmp_header = struct.pack('<IiiHHIIiiII',
        40,        # header size
        size,      # width
        size * 2,  # height (doubled for ICO)
        1,         # planes
        32,        # bits per pixel
        0,         # compression (none)
        0,         # image size
        0,         # x pixels per meter
        0,         # y pixels per meter
        0,         # colors in palette
        0          # important colors
    )

    # Pixel data (BGRA)
    pixels = b''
    for y in range(size):
        for x in range(size):
            # Simple spider web pattern
            cx, cy = size // 2, size // 2
            dist = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
            if dist < size * 0.3 or (dist < size * 0.5 and (x == cx or y == cy)):
                pixels += struct.pack('BBBB', b, g, r, a)  # BGRA
            else:
                pixels += struct.pack('BBBB', 0, 0, 0, 0)  # Transparent

    # AND mask (1 bit per pixel)
    and_mask = b'\x00' * (((size + 31) // 32) * 4) * size

    image_data = bmp_header + pixels + and_mask

    # ICO header
    ico_header = struct.pack('<HHH', 0, 1, 1)  # reserved, type=ICO, count=1

    # ICO directory entry
    ico_entry = struct.pack('<BBBBHHII',
        size if size < 256 else 0,  # width
        size if size < 256 else 0,  # height
        0,         # colors in palette
        0,         # reserved
        1,         # planes
        32,        # bits per pixel
        len(image_data),  # size of image data
        6 + 16     # offset (header + 1 entry)
    )

    with open(filepath, 'wb') as f:
        f.write(ico_header)
        f.write(ico_entry)
        f.write(image_data)

if __name__ == '__main__':
    icons_dir = os.path.dirname(os.path.abspath(__file__))

    # Generate PNG icons
    create_png(32, 32, os.path.join(icons_dir, '32x32.png'))
    create_png(128, 128, os.path.join(icons_dir, '128x128.png'))
    create_png(256, 256, os.path.join(icons_dir, '128x128@2x.png'))
    create_png(256, 256, os.path.join(icons_dir, 'icon.png'))

    # Generate ICO
    create_ico(256, os.path.join(icons_dir, 'icon.ico'))

    # Create ICNS (just copy the 256x256 PNG for now - macOS will accept it)
    import shutil
    shutil.copy(os.path.join(icons_dir, '128x128@2x.png'), os.path.join(icons_dir, 'icon.icns'))

    print("Icons generated successfully!")
