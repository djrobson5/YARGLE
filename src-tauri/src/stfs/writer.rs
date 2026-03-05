use std::fs;
use std::io::Cursor;

const DISPLAY_NAME_OFFSET: u64 = 0x0411;
const DISPLAY_NAME_MAX_BYTES: usize = 0x0100;
const DESCRIPTION_OFFSET: u64 = 0x0D11;
const DESCRIPTION_MAX_BYTES: usize = 0x0100;
const THUMBNAIL_SIZE_OFFSET: u64 = 0x1712;
const THUMBNAIL_DATA_OFFSET: u64 = 0x171A;
const THUMBNAIL_MAX_SIZE: usize = 0x4000;
const BLOCK_SIZE: usize = 0x1000;

/// Write a UTF-16 BE string at a fixed offset, null-padding the remainder
fn write_utf16be_string(data: &mut [u8], offset: usize, max_bytes: usize, value: &str) {
    let utf16: Vec<u16> = value.encode_utf16().collect();
    let max_chars = max_bytes / 2;

    // Zero-fill the region first
    for i in 0..max_bytes {
        if offset + i < data.len() {
            data[offset + i] = 0;
        }
    }

    // Write UTF-16 BE chars
    for (i, &code_unit) in utf16.iter().enumerate() {
        if i >= max_chars {
            break;
        }
        let bytes = code_unit.to_be_bytes();
        let pos = offset + i * 2;
        if pos + 1 < data.len() {
            data[pos] = bytes[0];
            data[pos + 1] = bytes[1];
        }
    }
}

/// Write display name to STFS header
pub fn write_display_name(data: &mut Vec<u8>, name: &str) {
    write_utf16be_string(data, DISPLAY_NAME_OFFSET as usize, DISPLAY_NAME_MAX_BYTES, name);
}

/// Write description to STFS header
pub fn write_description(data: &mut Vec<u8>, desc: &str) {
    write_utf16be_string(data, DESCRIPTION_OFFSET as usize, DESCRIPTION_MAX_BYTES, desc);
}

/// Write thumbnail image data (must be PNG or JPEG, <=16384 bytes)
pub fn write_thumbnail(data: &mut Vec<u8>, image_data: &[u8]) -> Result<(), String> {
    if image_data.len() > THUMBNAIL_MAX_SIZE {
        return Err(format!(
            "Thumbnail too large: {} bytes (max {})",
            image_data.len(),
            THUMBNAIL_MAX_SIZE
        ));
    }

    let offset = THUMBNAIL_DATA_OFFSET as usize;
    let size_offset = THUMBNAIL_SIZE_OFFSET as usize;

    // Write size (u32 BE)
    let size_bytes = (image_data.len() as u32).to_be_bytes();
    data[size_offset..size_offset + 4].copy_from_slice(&size_bytes);

    // Write the same size to title thumbnail size (they share the image data in v1)
    // Actually title thumbnail is separate - just write the content thumbnail
    // Also write the duplicate size field at 0x1716
    let title_thumb_size_offset = 0x1716usize;
    data[title_thumb_size_offset..title_thumb_size_offset + 4].copy_from_slice(&size_bytes);

    // Zero-fill the thumbnail region first
    for i in 0..THUMBNAIL_MAX_SIZE {
        if offset + i < data.len() {
            data[offset + i] = 0;
        }
    }

    // Write new thumbnail data
    data[offset..offset + image_data.len()].copy_from_slice(image_data);

    Ok(())
}

/// Write modified DTA content back to the STFS package
/// block_offsets: file offsets of each data block that holds the DTA file
/// original_size: the original file size from the file table entry
pub fn write_dta_content(
    data: &mut Vec<u8>,
    new_dta: &[u8],
    block_offsets: &[u64],
    original_size: u32,
    file_entry_offset: Option<u64>,
) -> Result<(), String> {
    if new_dta.len() > original_size as usize {
        // Check if it fits within allocated blocks
        let total_capacity = block_offsets.len() * BLOCK_SIZE;
        if new_dta.len() > total_capacity {
            return Err(format!(
                "Modified DTA ({} bytes) exceeds allocated block capacity ({} bytes). \
                 Cannot grow beyond original allocation without block reallocation.",
                new_dta.len(),
                total_capacity
            ));
        }
    }

    // Write data block by block
    let mut remaining = new_dta;
    for &block_offset in block_offsets {
        let offset = block_offset as usize;
        let write_size = remaining.len().min(BLOCK_SIZE);

        if offset + BLOCK_SIZE > data.len() {
            return Err(format!("Block offset 0x{:X} out of bounds", offset));
        }

        // Zero the block first
        for i in 0..BLOCK_SIZE {
            data[offset + i] = 0;
        }

        // Write content
        data[offset..offset + write_size].copy_from_slice(&remaining[..write_size]);
        remaining = &remaining[write_size..];

        if remaining.is_empty() {
            break;
        }
    }

    // Update file size in file table entry if provided
    if let Some(entry_offset) = file_entry_offset {
        let size_offset = entry_offset as usize + 0x34;
        let new_size = (new_dta.len() as u32).to_be_bytes();
        if size_offset + 4 <= data.len() {
            data[size_offset..size_offset + 4].copy_from_slice(&new_size);
        }
    }

    Ok(())
}

/// Write file content back to the same block offsets in-place (no size change)
pub fn write_file_content_inplace(
    data: &mut Vec<u8>,
    content: &[u8],
    block_offsets: &[u64],
) -> Result<(), String> {
    let mut remaining = content;
    for &block_offset in block_offsets {
        let offset = block_offset as usize;
        let write_size = remaining.len().min(BLOCK_SIZE);

        if offset + write_size > data.len() {
            return Err(format!("Block offset 0x{:X} out of bounds", offset));
        }

        data[offset..offset + write_size].copy_from_slice(&remaining[..write_size]);
        remaining = &remaining[write_size..];

        if remaining.is_empty() {
            break;
        }
    }

    Ok(())
}

/// Save modified data back to file
pub fn save_to_file(path: &str, data: &[u8]) -> Result<(), String> {
    fs::write(path, data).map_err(|e| format!("Failed to write file: {}", e))
}

/// Resize an image to fit within the 16KB header thumbnail budget.
/// Tries progressively smaller sizes as JPEG to maximize quality.
pub fn resize_thumbnail(image_data: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(image_data)
        .map_err(|e| format!("Failed to load image: {}", e))?;

    // Try sizes from largest to smallest — pick the biggest that fits in 16KB
    for &size in &[256u32, 192, 128, 64] {
        let resized = img.resize(size, size, image::imageops::FilterType::Lanczos3);

        let mut buf = Vec::new();
        let mut cursor = Cursor::new(&mut buf);
        resized
            .write_to(&mut cursor, image::ImageFormat::Jpeg)
            .map_err(|e| format!("Failed to encode JPEG: {}", e))?;

        if buf.len() <= THUMBNAIL_MAX_SIZE {
            return Ok(buf);
        }
    }

    Err("Image cannot be compressed to fit within 16384 bytes".into())
}
