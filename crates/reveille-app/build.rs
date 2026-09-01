// SPDX-License-Identifier: GPL-2.0-only

use std::fs;
use std::path::Path;

fn main() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo provides manifest path");
    let icon_directory = Path::new(&manifest).join("icons");
    fs::create_dir_all(&icon_directory).expect("create generated icon directory");
    fs::write(icon_directory.join("icon.ico"), development_icon()).expect("write Windows icon");
    fs::write(icon_directory.join("icon.png"), development_png()).expect("write macOS icon");
    tauri_build::build();
}

fn pixel(x: usize, y: usize, side: usize) -> (u8, u8, u8) {
    let border = x < 3 || y < 3 || x >= side - 3 || y >= side - 3;
    let slash = x.abs_diff(y) < 3 || x + y > 43 && x + y < 49;
    if border || slash {
        (214, 173, 98)
    } else {
        (33, 39, 29)
    }
}

fn development_icon() -> Vec<u8> {
    const SIDE: usize = 32;
    const SIDE_I32: i32 = 32;
    const BITMAP_BYTES: usize = SIDE * SIDE * 4;
    const BITMAP_BYTES_U32: u32 = 32 * 32 * 4;
    const MASK_BYTES: usize = SIDE * 4;
    let image_bytes = 40 + BITMAP_BYTES + MASK_BYTES;
    let mut icon = Vec::with_capacity(22 + image_bytes);
    icon.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    icon.extend_from_slice(&[32, 32, 0, 0, 1, 0, 32, 0]);
    icon.extend_from_slice(&(40_u32 + BITMAP_BYTES_U32 + 128).to_le_bytes());
    icon.extend_from_slice(&22_u32.to_le_bytes());
    icon.extend_from_slice(&40_u32.to_le_bytes());
    icon.extend_from_slice(&SIDE_I32.to_le_bytes());
    icon.extend_from_slice(&(SIDE_I32 * 2).to_le_bytes());
    icon.extend_from_slice(&1_u16.to_le_bytes());
    icon.extend_from_slice(&32_u16.to_le_bytes());
    icon.extend_from_slice(&0_u32.to_le_bytes());
    icon.extend_from_slice(&BITMAP_BYTES_U32.to_le_bytes());
    icon.extend_from_slice(&[0; 16]);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let (red, green, blue) = pixel(x, y, SIDE);
            icon.extend_from_slice(&[blue, green, red, 255]);
        }
    }
    icon.extend(std::iter::repeat_n(0, MASK_BYTES));
    icon
}

/// Minimal 32×32 RGBA PNG so `tauri build --bundles app/dmg` has an icon on macOS.
fn development_png() -> Vec<u8> {
    const SIDE: usize = 32;
    let mut raw = Vec::with_capacity(SIDE * (1 + SIDE * 4));
    for y in 0..SIDE {
        raw.push(0);
        for x in 0..SIDE {
            let (red, green, blue) = pixel(x, y, SIDE);
            raw.extend_from_slice(&[red, green, blue, 255]);
        }
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(png_chunk(*b"IHDR", &ihdr_rgba(SIDE)));
    png.extend(png_chunk(*b"IDAT", &zlib_store(&raw)));
    png.extend(png_chunk(*b"IEND", &[]));
    png
}

fn ihdr_rgba(side: usize) -> [u8; 13] {
    let side = u32::try_from(side).expect("development PNG side fits u32");
    let mut header = [0_u8; 13];
    header[..4].copy_from_slice(&side.to_be_bytes());
    header[4..8].copy_from_slice(&side.to_be_bytes());
    header[8] = 8;
    header[9] = 6;
    header
}

fn png_chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
    let length = u32::try_from(data.len()).expect("development PNG chunk fits u32");
    let mut chunk = Vec::with_capacity(12 + data.len());
    chunk.extend_from_slice(&length.to_be_bytes());
    chunk.extend_from_slice(&kind);
    chunk.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(&kind);
    crc_input.extend_from_slice(data);
    chunk.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    chunk
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01, 0x01];
    let len = u16::try_from(data.len()).expect("development PNG fits one stored block");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1_u32;
    let mut b = 0_u32;
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xEDB8_8320
            };
        }
    }
    !crc
}
