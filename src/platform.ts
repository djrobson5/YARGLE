// OS-specific names for UI text, so macOS says Finder/Trash and Windows says
// File Explorer/Recycle Bin.
export const IS_MAC = /Mac/i.test(navigator.userAgent);

export const FILE_MANAGER = IS_MAC ? "Finder" : "File Explorer";
export const FILE_MANAGER_SHORT = IS_MAC ? "Finder" : "Explorer";
export const TRASH = IS_MAC ? "Trash" : "Recycle Bin";
