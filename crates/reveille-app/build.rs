// SPDX-License-Identifier: GPL-2.0-only

use std::fs;
use std::path::Path;

fn main() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo provides manifest path");
    let icon_directory = Path::new(&manifest).join("icons");
    fs::create_dir_all(&icon_directory).expect("create generated icon directory");
    let icon = icon_directory.join("icon.ico");
    fs::write(&icon, development_icon()).expect("write generated development icon");
    tauri_build::build();
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
            let border = x < 3 || y < 3 || x >= SIDE - 3 || y >= SIDE - 3;
            let slash = x.abs_diff(y) < 3 || x + y > 43 && x + y < 49;
            let (red, green, blue) = if border || slash {
                (214, 173, 98)
            } else {
                (33, 39, 29)
            };
            icon.extend_from_slice(&[blue, green, red, 255]);
        }
    }
    icon.extend(std::iter::repeat_n(0, MASK_BYTES));
    icon
}
