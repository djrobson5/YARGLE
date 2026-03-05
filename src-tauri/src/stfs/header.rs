use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Cursor, Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq)]
pub enum PackageType {
    CON,
    LIVE,
    PIRS,
}

#[derive(Debug, Clone)]
pub struct StfsHeader {
    pub package_type: PackageType,
    pub display_name: String,
    pub description: String,
    pub title_name: String,
    pub content_type: u32,
    pub title_id: u32,
    pub thumbnail_size: u32,
    pub thumbnail_data: Vec<u8>,
    pub title_thumbnail_size: u32,
    pub title_thumbnail_data: Vec<u8>,
}

const DISPLAY_NAME_OFFSET: u64 = 0x0411;
const DISPLAY_NAME_SIZE: usize = 0x0100; // 128 UTF-16 chars (256 bytes)
const DESCRIPTION_OFFSET: u64 = 0x0D11;
const DESCRIPTION_SIZE: usize = 0x0100;
const TITLE_NAME_OFFSET: u64 = 0x1691;
const TITLE_NAME_SIZE: usize = 0x0080; // 64 UTF-16 chars (128 bytes)
const CONTENT_TYPE_OFFSET: u64 = 0x0344;
const TITLE_ID_OFFSET: u64 = 0x0360;
const THUMBNAIL_SIZE_OFFSET: u64 = 0x1712;
const THUMBNAIL_DATA_OFFSET: u64 = 0x171A;
const THUMBNAIL_MAX_SIZE: usize = 0x4000;
const TITLE_THUMBNAIL_SIZE_OFFSET: u64 = 0x1716;
const TITLE_THUMBNAIL_DATA_OFFSET: u64 = 0x571A;

fn read_utf16be_string(cursor: &mut Cursor<&[u8]>, offset: u64, max_bytes: usize) -> Result<String, String> {
    cursor.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; max_bytes];
    cursor.read_exact(&mut buf).map_err(|e| e.to_string())?;

    let mut chars = Vec::new();
    for chunk in buf.chunks(2) {
        if chunk.len() < 2 {
            break;
        }
        let code_unit = u16::from_be_bytes([chunk[0], chunk[1]]);
        if code_unit == 0 {
            break;
        }
        chars.push(code_unit);
    }
    String::from_utf16(&chars).map_err(|e| e.to_string())
}

/// Lightweight header parse: only reads ~6KB for the file list summary.
/// Skips thumbnail/title-thumbnail data entirely.
pub fn parse_header_summary(data: &[u8]) -> Result<StfsHeader, String> {
    if data.len() < 0x171A {
        return Err("File too small to be a valid STFS package".into());
    }

    let mut cursor = Cursor::new(data);

    let magic = &data[0..4];
    let package_type = match magic {
        b"CON " => PackageType::CON,
        b"LIVE" => PackageType::LIVE,
        b"PIRS" => PackageType::PIRS,
        _ => return Err(format!("Unknown package type: {:?}", &magic[..3])),
    };

    let display_name = read_utf16be_string(&mut cursor, DISPLAY_NAME_OFFSET, DISPLAY_NAME_SIZE)?;
    let description = read_utf16be_string(&mut cursor, DESCRIPTION_OFFSET, DESCRIPTION_SIZE)?;
    let title_name = read_utf16be_string(&mut cursor, TITLE_NAME_OFFSET, TITLE_NAME_SIZE)?;

    cursor.seek(SeekFrom::Start(THUMBNAIL_SIZE_OFFSET)).map_err(|e| e.to_string())?;
    let thumbnail_size = cursor.read_u32::<BigEndian>().map_err(|e| e.to_string())?;

    Ok(StfsHeader {
        package_type,
        display_name,
        description,
        title_name,
        content_type: 0,
        title_id: 0,
        thumbnail_size,
        thumbnail_data: Vec::new(),
        title_thumbnail_size: 0,
        title_thumbnail_data: Vec::new(),
    })
}

pub fn parse_header(data: &[u8]) -> Result<StfsHeader, String> {
    if data.len() < 0x971A {
        return Err("File too small to be a valid STFS package".into());
    }

    let mut cursor = Cursor::new(data);

    // Magic bytes
    let magic = &data[0..4];
    let package_type = match magic {
        b"CON " => PackageType::CON,
        b"LIVE" => PackageType::LIVE,
        b"PIRS" => PackageType::PIRS,
        _ => return Err(format!("Unknown package type: {:?}", &magic[..3])),
    };

    let display_name = read_utf16be_string(&mut cursor, DISPLAY_NAME_OFFSET, DISPLAY_NAME_SIZE)?;
    let description = read_utf16be_string(&mut cursor, DESCRIPTION_OFFSET, DESCRIPTION_SIZE)?;
    let title_name = read_utf16be_string(&mut cursor, TITLE_NAME_OFFSET, TITLE_NAME_SIZE)?;

    // Content type
    cursor.seek(SeekFrom::Start(CONTENT_TYPE_OFFSET)).map_err(|e| e.to_string())?;
    let content_type = cursor.read_u32::<BigEndian>().map_err(|e| e.to_string())?;

    // Title ID
    cursor.seek(SeekFrom::Start(TITLE_ID_OFFSET)).map_err(|e| e.to_string())?;
    let title_id = cursor.read_u32::<BigEndian>().map_err(|e| e.to_string())?;

    // Thumbnail
    cursor.seek(SeekFrom::Start(THUMBNAIL_SIZE_OFFSET)).map_err(|e| e.to_string())?;
    let thumbnail_size = cursor.read_u32::<BigEndian>().map_err(|e| e.to_string())?;
    let thumb_read_size = (thumbnail_size as usize).min(THUMBNAIL_MAX_SIZE);
    cursor.seek(SeekFrom::Start(THUMBNAIL_DATA_OFFSET)).map_err(|e| e.to_string())?;
    let mut thumbnail_data = vec![0u8; thumb_read_size];
    cursor.read_exact(&mut thumbnail_data).map_err(|e| e.to_string())?;

    // Title thumbnail
    cursor.seek(SeekFrom::Start(TITLE_THUMBNAIL_SIZE_OFFSET)).map_err(|e| e.to_string())?;
    let title_thumbnail_size = cursor.read_u32::<BigEndian>().map_err(|e| e.to_string())?;
    let title_thumb_read_size = (title_thumbnail_size as usize).min(THUMBNAIL_MAX_SIZE);
    cursor.seek(SeekFrom::Start(TITLE_THUMBNAIL_DATA_OFFSET)).map_err(|e| e.to_string())?;
    let mut title_thumbnail_data = vec![0u8; title_thumb_read_size];
    cursor.read_exact(&mut title_thumbnail_data).map_err(|e| e.to_string())?;

    Ok(StfsHeader {
        package_type,
        display_name,
        description,
        title_name,
        content_type,
        title_id,
        thumbnail_size,
        thumbnail_data,
        title_thumbnail_size,
        title_thumbnail_data,
    })
}
