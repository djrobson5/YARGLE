/// Decoder for Xbox 360 `_keep.png_xbox` album art textures.
///
/// Format: 32-byte HMX header + byte-swapped DXT1 or DXT5 texture data.
///
/// Header layout (from empirical analysis + C3 CON Tools):
///   byte 1: BPP indicator (0x04 = DXT1/4bpp, 0x08 = DXT5/8bpp)
///   bytes 8-9: width as u16 BE in 256-unit blocks (0x0100 = 256, 0x0200 = 512)
///   bytes 10-11: height (similar encoding, may have flags in low bits)

const HMX_HEADER_SIZE: usize = 32;

pub struct DecodedTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn decode_png_xbox(data: &[u8]) -> Result<DecodedTexture, String> {
    if data.len() < HMX_HEADER_SIZE + 8 {
        return Err("File too small for HMX texture".into());
    }

    let header = &data[..HMX_HEADER_SIZE];
    let bpp_indicator = header[1];

    let is_dxt5 = match bpp_indicator {
        0x04 => false, // DXT1
        0x08 => true,  // DXT5
        _ => {
            // Fallback: guess from file size
            let tex_size = data.len() - HMX_HEADER_SIZE;
            // DXT5 is 1 byte/pixel, DXT1 is 0.5 bytes/pixel
            // For 256x256: DXT1=32768, DXT5=65536
            // For 512x512: DXT1=131072, DXT5=262144
            tex_size > 50000
        }
    };

    // Parse dimensions from header bytes 8-9
    let width_raw = u16::from_be_bytes([header[8], header[9]]);
    let height_raw = u16::from_be_bytes([header[10], header[11]]);

    // Dimensions are encoded as value * 256 in some cases, or directly
    let width = if width_raw <= 8 {
        (width_raw as u32) * 256
    } else {
        width_raw as u32
    };
    let height_clean = height_raw & 0x7FFF; // mask off possible flags
    let height = if height_clean <= 8 {
        (height_clean as u32) * 256
    } else {
        height_clean as u32
    };

    // Validate dimensions
    if width == 0 || height == 0 || width > 2048 || height > 2048 {
        // Fallback: guess from data size
        return decode_with_guessed_dimensions(data, is_dxt5);
    }

    decode_texture_data(data, width, height, is_dxt5)
}

fn decode_with_guessed_dimensions(data: &[u8], is_dxt5: bool) -> Result<DecodedTexture, String> {
    let tex_size = data.len() - HMX_HEADER_SIZE;

    // Try common sizes: 512x512, 256x256
    let candidates: &[(u32, u32)] = &[(512, 512), (256, 256), (1024, 1024), (128, 128)];

    for &(w, h) in candidates {
        let expected = if is_dxt5 {
            (w * h) as usize // DXT5 = 1 byte/pixel
        } else {
            (w * h / 2) as usize // DXT1 = 0.5 bytes/pixel
        };
        // Allow for mipmaps (data can be larger than base level)
        if tex_size >= expected {
            return decode_texture_data(data, w, h, is_dxt5);
        }
    }

    Err(format!(
        "Cannot determine texture dimensions (data size: {}, dxt5: {})",
        tex_size, is_dxt5
    ))
}

fn decode_texture_data(
    data: &[u8],
    width: u32,
    height: u32,
    is_dxt5: bool,
) -> Result<DecodedTexture, String> {
    // Calculate expected base-level texture data size
    let base_size = if is_dxt5 {
        (width * height) as usize
    } else {
        (width * height / 2) as usize
    };

    if data.len() < HMX_HEADER_SIZE + base_size {
        return Err(format!(
            "Not enough texture data: have {}, need {} for {}x{} {}",
            data.len() - HMX_HEADER_SIZE,
            base_size,
            width,
            height,
            if is_dxt5 { "DXT5" } else { "DXT1" }
        ));
    }

    // Copy only the base mipmap level and byte-swap
    let mut tex_data = data[HMX_HEADER_SIZE..HMX_HEADER_SIZE + base_size].to_vec();

    // Xbox 360 big-endian → little-endian: swap each 16-bit pair
    for chunk in tex_data.chunks_exact_mut(2) {
        chunk.swap(0, 1);
    }

    // Decode DXT
    let pixel_count = (width * height) as usize;
    let mut pixels = vec![0u32; pixel_count];

    if is_dxt5 {
        texture2ddecoder::decode_bc3(&tex_data, width as usize, height as usize, &mut pixels)
            .map_err(|e| format!("DXT5 decode error: {}", e))?;
    } else {
        texture2ddecoder::decode_bc1(&tex_data, width as usize, height as usize, &mut pixels)
            .map_err(|e| format!("DXT1 decode error: {}", e))?;
    }

    // Convert u32 BGRA pixels to RGBA u8 vec
    // texture2ddecoder outputs BGRA packed as u32 little-endian
    let rgba: Vec<u8> = pixels
        .iter()
        .flat_map(|&p| {
            let [b, g, r, a] = p.to_le_bytes();
            [r, g, b, a]
        })
        .collect();

    Ok(DecodedTexture {
        width,
        height,
        rgba,
    })
}

/// Encode decoded RGBA pixels as a PNG in memory
pub fn texture_to_png(tex: &DecodedTexture) -> Result<Vec<u8>, String> {
    use image::{ImageBuffer, RgbaImage};
    use std::io::Cursor;

    let img: RgbaImage = ImageBuffer::from_raw(tex.width, tex.height, tex.rgba.clone())
        .ok_or("Failed to create image buffer")?;

    let mut png_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut png_bytes);
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("PNG encode error: {}", e))?;

    Ok(png_bytes)
}
