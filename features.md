# YARGLE Feature Ideas

## Library Management
- [x] **Duplicate detection** — scan for duplicate songs (same shortname or similar artist+title) across a folder, help clean up libraries
- [x] **Batch rename** — rename CON files on disk based on metadata (e.g., `Artist - Title_rb3con`) instead of cryptic filenames
- [x] **Multi-folder support / library index** — remember multiple song folders, search across all of them

## Metadata & Quality
- [x] **Batch metadata editor** — select multiple songs, edit a shared field (e.g., fix a charter name across all their songs)
- [ ] **Missing metadata warnings** — flag songs with empty artist, no album art, zero difficulty ranks, etc.
- [x] **DTA validator** — catch common DTA issues (bad encoding, missing required fields) that cause YARG to skip songs
- [ ] **Preview audio** — play a short clip of the decrypted MOGG so you can identify mystery songs

## Import / Export
- [ ] **CON to YARG conversion** — extract chart + audio to the loose-file format YARG also supports, or vice versa
- [ ] **Setlist export** — export library as a CSV/spreadsheet (artist, title, charter, difficulty, etc.) for sharing or tracking
- [ ] **Import from spreadsheet** — batch-update metadata from a CSV

## YARG Integration
- [ ] **Song cache viewer** — read YARG's song cache to show which songs it actually sees vs. what's on disk
- [ ] **Broken song finder** — cross-reference CON files against YARG's scan errors to find songs that fail to load
- [x] **Auto-organize** — sort CON files into subfolders by artist, genre, or charter

## Nice-to-haves
- [ ] **Dark/light theme toggle**
- [ ] **Drag-and-drop** files or folders onto the window
- [x] **Chart preview** — render a simple note highway visualization from the .mid
