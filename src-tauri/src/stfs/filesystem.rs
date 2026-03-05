use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Cursor, Seek, SeekFrom};

const VOLUME_DESCRIPTOR_OFFSET: u64 = 0x0379;
const BLOCK_SIZE: usize = 0x1000; // 4096 bytes

/// Base offset where the block region starts (hash tables + data blocks).
/// For CON packages with metadata v2 (title thumbnails), this is 0xB000.
const BLOCK_REGION_BASE: usize = 0xB000;

#[derive(Debug, Clone)]
pub struct VolumeDescriptor {
    pub block_separation: u8,
    pub file_table_block: u32,
    pub file_table_block_count: u16,
    pub total_allocated_blocks: u32,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub is_directory: bool,
    pub num_blocks: u32,
    pub starting_block: u32,
    pub path_indicator: u16,
    pub file_size: u32,
    pub update_time: u32,
    pub access_time: u32,
}

#[derive(Debug)]
pub struct StfsFilesystem {
    pub volume_descriptor: VolumeDescriptor,
    pub files: Vec<FileEntry>,
    data: Vec<u8>,
}

impl StfsFilesystem {
    pub fn parse(data: Vec<u8>) -> Result<Self, String> {
        let vd = parse_volume_descriptor(&data)?;
        let files = parse_file_table(&data, &vd)?;
        Ok(StfsFilesystem {
            volume_descriptor: vd,
            files,
            data,
        })
    }

    /// Extract file content by following the block chain
    pub fn extract_file(&self, entry: &FileEntry) -> Result<Vec<u8>, String> {
        if entry.is_directory {
            return Err("Cannot extract a directory".into());
        }

        let mut result = Vec::with_capacity(entry.file_size as usize);
        let mut current_block = entry.starting_block;
        let mut remaining = entry.file_size as usize;

        for _ in 0..entry.num_blocks {
            if remaining == 0 {
                break;
            }

            let offset = block_to_offset(current_block, &self.volume_descriptor);
            let read_size = remaining.min(BLOCK_SIZE);

            if offset + read_size > self.data.len() {
                return Err(format!(
                    "Block {} offset 0x{:X} exceeds file size",
                    current_block, offset
                ));
            }

            result.extend_from_slice(&self.data[offset..offset + read_size]);
            remaining -= read_size;

            if remaining > 0 {
                current_block =
                    get_next_block(&self.data, current_block, &self.volume_descriptor)?;
                if current_block == 0xFFFFFF {
                    break;
                }
            }
        }

        result.truncate(entry.file_size as usize);
        Ok(result)
    }

    /// Find and extract songs.dta from the package
    pub fn extract_songs_dta(&self) -> Result<(Vec<u8>, FileEntry), String> {
        let songs_dir_idx = self
            .files
            .iter()
            .position(|f| f.is_directory && f.name.eq_ignore_ascii_case("songs"))
            .ok_or("No 'songs' directory found in package")?;

        let dta_entry = self
            .files
            .iter()
            .find(|f| {
                !f.is_directory
                    && f.name.eq_ignore_ascii_case("songs.dta")
                    && f.path_indicator == songs_dir_idx as u16
            })
            .ok_or("No 'songs.dta' found in songs directory")?;

        let content = self.extract_file(dta_entry)?;
        Ok((content, dta_entry.clone()))
    }

    /// Find and extract album art (_keep.png_xbox) from the package
    pub fn extract_album_art(&self) -> Result<Vec<u8>, String> {
        let gen_dir_idx = self
            .files
            .iter()
            .position(|f| f.is_directory && f.name.eq_ignore_ascii_case("gen"));

        let art_entry = self.files.iter().find(|f| {
            !f.is_directory
                && f.name.ends_with("_keep.png_xbox")
                && (gen_dir_idx.is_none() || f.path_indicator == gen_dir_idx.unwrap() as u16)
        });

        match art_entry {
            Some(entry) => self.extract_file(entry),
            None => Err("No album art (_keep.png_xbox) found in package".into()),
        }
    }

    /// Get the file offset and size info for songs.dta (for write-back)
    pub fn get_songs_dta_location(&self) -> Result<(FileEntry, Vec<u64>), String> {
        let (_, entry) = self.extract_songs_dta()?;

        let mut offsets = Vec::new();
        let mut current_block = entry.starting_block;

        for _ in 0..entry.num_blocks {
            let offset = block_to_offset(current_block, &self.volume_descriptor);
            offsets.push(offset as u64);

            current_block =
                match get_next_block(&self.data, current_block, &self.volume_descriptor) {
                    Ok(next) if next != 0xFFFFFF => next,
                    _ => break,
                };
        }

        Ok((entry, offsets))
    }
}

fn parse_volume_descriptor(data: &[u8]) -> Result<VolumeDescriptor, String> {
    if data.len() < (VOLUME_DESCRIPTOR_OFFSET as usize + 0x24) {
        return Err("File too small for volume descriptor".into());
    }

    let mut cursor = Cursor::new(data);
    cursor
        .seek(SeekFrom::Start(VOLUME_DESCRIPTOR_OFFSET))
        .map_err(|e| e.to_string())?;

    let _descriptor_size = cursor.read_u8().map_err(|e| e.to_string())?;
    let _reserved = cursor.read_u8().map_err(|e| e.to_string())?;
    let block_separation = cursor.read_u8().map_err(|e| e.to_string())?;

    let file_table_block_count = {
        let lo = cursor.read_u8().map_err(|e| e.to_string())? as u16;
        let hi = cursor.read_u8().map_err(|e| e.to_string())? as u16;
        (hi << 8) | lo
    };

    let b0 = cursor.read_u8().map_err(|e| e.to_string())? as u32;
    let b1 = cursor.read_u8().map_err(|e| e.to_string())? as u32;
    let b2 = cursor.read_u8().map_err(|e| e.to_string())? as u32;
    let file_table_block = (b0 << 16) | (b1 << 8) | b2;

    cursor
        .seek(SeekFrom::Start(VOLUME_DESCRIPTOR_OFFSET + 0x22))
        .map_err(|e| e.to_string())?;
    let total_high = cursor.read_u8().map_err(|e| e.to_string())? as u32;
    let total_low = cursor.read_u16::<BigEndian>().map_err(|e| e.to_string())? as u32;
    let total_allocated_blocks = (total_high << 16) | total_low;

    Ok(VolumeDescriptor {
        block_separation,
        file_table_block,
        file_table_block_count,
        total_allocated_blocks,
    })
}

/// Convert a data block number to a file offset.
///
/// STFS block layout (from BLOCK_REGION_BASE):
///   [L0 Hash Table 0] [Data 0] [Data 1] ... [Data 169]
///   [L1 Hash Table 0] [L0 Hash Table 1] [Data 170] ... [Data 339]
///   ...
///
/// For "female" packages (block_separation > 0), each hash table
/// occupies 2 consecutive blocks instead of 1.
fn block_to_offset(block: u32, _vd: &VolumeDescriptor) -> usize {
    let block = block as usize;

    // Compute the "backing block number" — the physical position in the block region,
    // accounting for interleaved hash tables.
    //
    // Layout: [L0_0][data 0-169][L1_0][L0_1][data 170-339][L0_2][data 340-509]...
    //
    // For blocks 0-169: skip 1 hash block (L0_0)
    // For blocks 170+: skip L0_0, L1_0, plus one additional L0 per 170-block group
    let backing = if block < 170 {
        block + 1
    } else {
        block + ((block - 170) / 170) + 3
    };

    BLOCK_REGION_BASE + backing * BLOCK_SIZE
}

/// Get the file offset of the L0 hash table that covers a given data block.
fn hash_table_offset_for_block(block: u32, _vd: &VolumeDescriptor) -> usize {
    let group = (block / 170) as usize;
    if group == 0 {
        // L0 hash table 0 is at the very start of the block region
        BLOCK_REGION_BASE
    } else {
        // L0 hash table for group N is at:
        // base + (1 + 170 + 1) + (group-1) * (1 + 170) blocks from start
        // = base + (172 + (group-1)*171) * BLOCK_SIZE
        // Simplified: position = 1 (L0_0) + 170 (data) + 1 (L1) + (group-1)*(170+1)
        let pos = 172 + (group - 1) * 171;
        BLOCK_REGION_BASE + pos * BLOCK_SIZE
    }
}

/// Get the next block in a chain from the L0 hash table
fn get_next_block(data: &[u8], block: u32, vd: &VolumeDescriptor) -> Result<u32, String> {
    let hash_table_index = (block % 170) as usize;

    let hash_table_start = hash_table_offset_for_block(block, vd);

    // Each hash entry is 0x18 (24) bytes: 20 bytes SHA1 + 1 byte status + 3 bytes next block
    let entry_offset = hash_table_start + hash_table_index * 0x18;

    if entry_offset + 0x18 > data.len() {
        return Err(format!(
            "Hash entry offset 0x{:X} out of bounds",
            entry_offset
        ));
    }

    let status = data[entry_offset + 0x14];
    let next_b0 = data[entry_offset + 0x15] as u32;
    let next_b1 = data[entry_offset + 0x16] as u32;
    let next_b2 = data[entry_offset + 0x17] as u32;
    let next_block = (next_b0 << 16) | (next_b1 << 8) | next_b2;

    if next_block == 0xFFFFFF || status == 0 {
        Ok(0xFFFFFF)
    } else {
        Ok(next_block)
    }
}

fn parse_file_table(data: &[u8], vd: &VolumeDescriptor) -> Result<Vec<FileEntry>, String> {
    let mut entries = Vec::new();
    let mut current_block = vd.file_table_block;

    for _table_block in 0..vd.file_table_block_count {
        let offset = block_to_offset(current_block, vd);

        if offset + BLOCK_SIZE > data.len() {
            break;
        }

        for i in 0..64 {
            let entry_offset = offset + i * 0x40;
            if entry_offset + 0x40 > data.len() {
                break;
            }

            let entry_data = &data[entry_offset..entry_offset + 0x40];

            let name_len = entry_data[0x28] & 0x3F;
            if name_len == 0 {
                continue;
            }

            let is_directory = (entry_data[0x28] & 0x80) != 0;

            let name_bytes = &entry_data[0..name_len as usize];
            let name = String::from_utf8_lossy(name_bytes).to_string();

            let num_blocks = (entry_data[0x29] as u32)
                | ((entry_data[0x2A] as u32) << 8)
                | ((entry_data[0x2B] as u32) << 16);

            let starting_block = (entry_data[0x2F] as u32)
                | ((entry_data[0x30] as u32) << 8)
                | ((entry_data[0x31] as u32) << 16);

            let path_indicator = u16::from_be_bytes([entry_data[0x32], entry_data[0x33]]);

            let file_size = u32::from_be_bytes([
                entry_data[0x34],
                entry_data[0x35],
                entry_data[0x36],
                entry_data[0x37],
            ]);

            let update_time = u32::from_be_bytes([
                entry_data[0x38],
                entry_data[0x39],
                entry_data[0x3A],
                entry_data[0x3B],
            ]);
            let access_time = u32::from_be_bytes([
                entry_data[0x3C],
                entry_data[0x3D],
                entry_data[0x3E],
                entry_data[0x3F],
            ]);

            entries.push(FileEntry {
                name,
                is_directory,
                num_blocks,
                starting_block,
                path_indicator,
                file_size,
                update_time,
                access_time,
            });
        }

        current_block = match get_next_block(data, current_block, vd) {
            Ok(next) if next != 0xFFFFFF => next,
            _ => break,
        };
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_con_file_entries() {
        // Try all CON files in the project root
        let con_files = [
            r"C:\Users\djrob\.gemini\antigravity\scratch\YARGLE\Gram Parsons - 1000 Wedding",
            r"C:\Users\djrob\.gemini\antigravity\scratch\YARGLE\Judas Priest - Diamonds and Rust_v2_rb3con",
            r"C:\Users\djrob\.gemini\antigravity\scratch\YARGLE\MazingerZRockanime",
            r"C:\Users\djrob\.gemini\antigravity\scratch\YARGLE\microchip_rb3con",
            r"C:\Users\djrob\.gemini\antigravity\scratch\YARGLE\My Chemical Romance - Blood [Hidden Track] (Jaded)_chps_rb3con",
        ];

        for path in &con_files {
            let data = match std::fs::read(path) {
                Ok(d) => d,
                Err(e) => {
                    println!("\n=== SKIPPING {} ===\nError: {}", path, e);
                    continue;
                }
            };

            println!("\n========================================");
            println!("FILE: {}", path);
            println!("  Size: {} bytes", data.len());
            println!("========================================");

            match StfsFilesystem::parse(data) {
                Ok(fs) => {
                    println!("Volume Descriptor: {:?}", fs.volume_descriptor);
                    println!("\nFile Entries ({} total):", fs.files.len());
                    println!("{:<4} {:<40} {:>6} {:>10} {:>8} {:>12}",
                        "Idx", "Name", "IsDir", "PathInd", "Blocks", "FileSize");
                    println!("{}", "-".repeat(90));
                    for (i, entry) in fs.files.iter().enumerate() {
                        println!("{:<4} {:<40} {:>6} {:>10} {:>8} {:>12}",
                            i,
                            entry.name,
                            entry.is_directory,
                            entry.path_indicator,
                            entry.num_blocks,
                            entry.file_size);
                    }
                }
                Err(e) => {
                    println!("  Parse error: {}", e);
                }
            }
        }
    }

    #[test]
    fn dump_png_xbox() {
        let path = r"C:\Users\djrob\.gemini\antigravity\scratch\YARGLE\My Chemical Romance - Blood [Hidden Track] (Jaded)_chps_rb3con";
        let data = std::fs::read(path).expect("Failed to read CON file");
        let fs = StfsFilesystem::parse(data).expect("Failed to parse STFS filesystem");

        // Find the _keep.png_xbox file
        let entry = fs
            .files
            .iter()
            .find(|f| f.name.contains("_keep.png_xbox"))
            .expect("No _keep.png_xbox file found in package");

        println!("Found: {} ({} bytes, {} blocks, starting block {})",
            entry.name, entry.file_size, entry.num_blocks, entry.starting_block);

        let content = fs.extract_file(entry).expect("Failed to extract file");

        println!("\nTotal extracted size: {} bytes", content.len());
        println!("\nFirst 64 bytes hex dump:");
        for (i, chunk) in content[..64.min(content.len())].chunks(16).enumerate() {
            // Hex portion
            let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
            // ASCII portion
            let ascii: String = chunk
                .iter()
                .map(|&b| if (0x20..=0x7E).contains(&b) { b as char } else { '.' })
                .collect();
            println!("{:08X}  {:<48}  |{}|", i * 16, hex.join(" "), ascii);
        }
    }

    #[test]
    fn dump_png_xbox_large() {
        let path = r"C:\Users\djrob\.gemini\antigravity\scratch\YARGLE\Gram Parsons - 1000 Wedding";
        let data = std::fs::read(path).expect("Failed to read CON file");
        let fs = StfsFilesystem::parse(data).expect("Failed to parse STFS filesystem");

        // Find the _keep.png_xbox file
        let entry = fs
            .files
            .iter()
            .find(|f| f.name.contains("_keep.png_xbox"))
            .expect("No _keep.png_xbox file found in package");

        println!("Found: {} ({} bytes, {} blocks, starting block {})",
            entry.name, entry.file_size, entry.num_blocks, entry.starting_block);

        let content = fs.extract_file(entry).expect("Failed to extract file");

        println!("\nTotal extracted size: {} bytes", content.len());
        println!("\nFirst 64 bytes hex dump:");
        for (i, chunk) in content[..64.min(content.len())].chunks(16).enumerate() {
            // Hex portion
            let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
            // ASCII portion
            let ascii: String = chunk
                .iter()
                .map(|&b| if (0x20..=0x7E).contains(&b) { b as char } else { '.' })
                .collect();
            println!("{:08X}  {:<48}  |{}|", i * 16, hex.join(" "), ascii);
        }
    }
}
