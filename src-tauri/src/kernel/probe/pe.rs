//! Reads `FILEVERSION` from the `VS_VERSIONINFO` resource of a PE file (`chrome.exe`).
//!
//! `chrome.exe --version` prints nothing on Windows (it is a GUI-subsystem program), so the
//! version is read from the file instead. Plain Rust over the file bytes: it runs, and is
//! tested, on every host. Every offset is bounds-checked; anything unexpected yields `None`.
//!
//! Layout references (Microsoft PE format specification):
//! - DOS header: `e_lfanew` (u32 at 0x3C) points to `PE\0\0`, followed by the 20-byte COFF
//!   header (`NumberOfSections` at +2, `SizeOfOptionalHeader` at +16).
//! - Optional header: magic 0x10B (PE32) or 0x20B (PE32+); `NumberOfRvaAndSizes` at +92 / +108,
//!   then the data directories (8 bytes each); index 2 is the resource table.
//! - Section headers (40 bytes): `VirtualSize` +8, `VirtualAddress` +12, `SizeOfRawData` +16,
//!   `PointerToRawData` +20.
//! - Resource directory (16 bytes, then 8-byte entries: `NumberOfNamedEntries` at +12,
//!   `NumberOfIdEntries` at +14): type → name → language. The high bit of an entry's offset
//!   marks a subdirectory; a leaf is `{ OffsetToData (RVA), Size, CodePage, Reserved }`.
//! - `VS_VERSIONINFO`: `wLength`, `wValueLength`, `wType`, the UTF-16 key `VS_VERSION_INFO\0`,
//!   padding to 32 bits, then `VS_FIXEDFILEINFO` (signature 0xFEEF04BD, `dwFileVersionMS` at +8,
//!   `dwFileVersionLS` at +12).

/// `RT_VERSION`.
const RT_VERSION: u32 = 16;
const VS_FFI_SIGNATURE: u32 = 0xFEEF_04BD;
const IMAGE_DIRECTORY_ENTRY_RESOURCE: usize = 2;
const SUBDIRECTORY: u32 = 0x8000_0000;

/// Whether `data` starts like a PE (DOS) image.
pub(super) fn is_pe(data: &[u8]) -> bool {
    data.starts_with(b"MZ")
}

/// `a.b.c.d` from the fixed file info, e.g. `150.0.7871.186`.
pub(super) fn file_version(data: &[u8]) -> Option<String> {
    let pe = Pe::parse(data)?;
    let resource = pe.resource_root()?;
    let info = resource.version_info()?;
    fixed_file_version(info)
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes(bytes.try_into().ok()?))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn usize_at(data: &[u8], offset: usize) -> Option<usize> {
    usize::try_from(u32_at(data, offset)?).ok()
}

struct Section {
    virtual_address: usize,
    virtual_size: usize,
    raw_size: usize,
    raw_offset: usize,
}

struct Pe<'a> {
    data: &'a [u8],
    sections: Vec<Section>,
    /// RVA and size of the resource table.
    resources: (usize, usize),
}

impl<'a> Pe<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        if !is_pe(data) {
            return None;
        }
        let pe_offset = usize_at(data, 0x3C)?;
        if data.get(pe_offset..pe_offset.checked_add(4)?)? != b"PE\0\0" {
            return None;
        }
        let coff = pe_offset + 4;
        let section_count = usize::from(u16_at(data, coff + 2)?);
        let optional_size = usize::from(u16_at(data, coff + 16)?);
        let optional = coff + 20;
        let directories = match u16_at(data, optional)? {
            0x10B => optional + 92,
            0x20B => optional + 108,
            _ => return None,
        };
        if usize_at(data, directories)? <= IMAGE_DIRECTORY_ENTRY_RESOURCE {
            return None;
        }
        let entry = directories + 4 + IMAGE_DIRECTORY_ENTRY_RESOURCE * 8;
        let resources = (usize_at(data, entry)?, usize_at(data, entry + 4)?);

        let table = optional.checked_add(optional_size)?;
        let sections = (0..section_count)
            .map(|i| {
                let header = table.checked_add(i.checked_mul(40)?)?;
                Some(Section {
                    virtual_size: usize_at(data, header + 8)?,
                    virtual_address: usize_at(data, header + 12)?,
                    raw_size: usize_at(data, header + 16)?,
                    raw_offset: usize_at(data, header + 20)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            data,
            sections,
            resources,
        })
    }

    /// The file bytes at `rva`, `len` long, if they lie in one section's raw data.
    fn slice(&self, rva: usize, len: usize) -> Option<&'a [u8]> {
        let section = self.sections.iter().find(|s| {
            let span = s.virtual_size.max(s.raw_size);
            rva >= s.virtual_address && rva - s.virtual_address < span
        })?;
        let delta = rva - section.virtual_address;
        if delta.checked_add(len)? > section.raw_size {
            return None;
        }
        let start = section.raw_offset.checked_add(delta)?;
        self.data.get(start..start.checked_add(len)?)
    }

    fn resource_root(&self) -> Option<Resources<'a, '_>> {
        let (rva, size) = self.resources;
        if rva == 0 || size == 0 {
            return None;
        }
        Some(Resources {
            pe: self,
            table: self.slice(rva, size)?,
        })
    }
}

struct Resources<'a, 'p> {
    pe: &'p Pe<'a>,
    /// The whole resource table; directory offsets are relative to its start.
    table: &'a [u8],
}

impl<'a> Resources<'a, '_> {
    /// The `VS_VERSIONINFO` block: the first name and first language under `RT_VERSION`.
    fn version_info(&self) -> Option<&'a [u8]> {
        let names = self.subdirectory(0, |id| id == RT_VERSION)?;
        let languages = self.subdirectory(names, |_| true)?;
        let leaf = self.entry(languages, |_| true)?;
        if leaf & SUBDIRECTORY != 0 {
            return None;
        }
        let leaf = usize::try_from(leaf).ok()?;
        let rva = usize_at(self.table, leaf)?;
        let size = usize_at(self.table, leaf + 4)?;
        self.pe.slice(rva, size)
    }

    fn subdirectory(&self, directory: usize, wanted: impl Fn(u32) -> bool) -> Option<usize> {
        let offset = self.entry(directory, wanted)?;
        if offset & SUBDIRECTORY == 0 {
            return None;
        }
        usize::try_from(offset & !SUBDIRECTORY).ok()
    }

    /// `OffsetToData` of the first entry of `directory` whose id or name field matches.
    /// Entry counts come from the file, so they are bounded by the table size.
    fn entry(&self, directory: usize, wanted: impl Fn(u32) -> bool) -> Option<u32> {
        let named = usize::from(u16_at(self.table, directory + 12)?);
        let ids = usize::from(u16_at(self.table, directory + 14)?);
        (0..named + ids)
            .map(|i| directory + 16 + i * 8)
            .take_while(|&entry| entry + 8 <= self.table.len())
            .find_map(|entry| {
                let id = u32_at(self.table, entry)?;
                wanted(id).then(|| u32_at(self.table, entry + 4))?
            })
    }
}

/// `FILEVERSION` of a `VS_VERSIONINFO` block.
fn fixed_file_version(info: &[u8]) -> Option<String> {
    const KEY: &str = "VS_VERSION_INFO";
    let value_length = usize::from(u16_at(info, 2)?);
    let key: Vec<u8> = KEY
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    if info.get(6..6 + key.len())? != key.as_slice() {
        return None;
    }
    // 6-byte header + 32-byte key = 38, padded to a 32-bit boundary.
    let fixed = (6 + key.len()).next_multiple_of(4);
    if value_length < 52 || u32_at(info, fixed)? != VS_FFI_SIGNATURE {
        return None;
    }
    let ms = u32_at(info, fixed + 8)?;
    let ls = u32_at(info, fixed + 12)?;
    Some(format!(
        "{}.{}.{}.{}",
        ms >> 16,
        ms & 0xFFFF,
        ls >> 16,
        ls & 0xFFFF
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Built with mingw-w64; see `tests/fixtures/pe/README.md`.
    const X64: &[u8] = include_bytes!("../../../tests/fixtures/pe/versioninfo-x64.bin");
    const X86: &[u8] = include_bytes!("../../../tests/fixtures/pe/versioninfo-x86.bin");

    #[test]
    fn reads_file_version_of_pe32_plus_and_pe32() {
        assert_eq!(file_version(X64).as_deref(), Some("150.0.7871.186"));
        assert_eq!(file_version(X86).as_deref(), Some("150.0.7871.186"));
    }

    #[test]
    fn rejects_non_pe_input() {
        for data in [
            &b""[..],
            b"MZ",
            b"\x7fELF\x02\x01\x01",
            b"MZ but not really a PE file",
        ] {
            assert_eq!(file_version(data), None);
        }
    }

    /// Every truncation of a valid file fails cleanly instead of panicking or reading out of
    /// bounds.
    #[test]
    fn truncated_files_never_panic() {
        for fixture in [X64, X86] {
            for len in 0..fixture.len() {
                let _ = file_version(&fixture[..len]);
            }
        }
    }

    /// Every single-byte corruption fails cleanly too (or still parses).
    #[test]
    fn corrupted_bytes_never_panic() {
        let mut data = X64.to_vec();
        for i in 0..data.len() {
            let original = data[i];
            for value in [0x00, 0x7F, 0xFF] {
                data[i] = value;
                let _ = file_version(&data);
            }
            data[i] = original;
        }
    }

    /// A real Windows binary: kernel32.dll always carries version information.
    #[cfg(windows)]
    #[test]
    fn reads_a_system_dll() {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let path = std::path::Path::new(&root).join(r"System32\kernel32.dll");
        let data = std::fs::read(&path).unwrap();
        let version = file_version(&data).unwrap();
        let major: u32 = version.split('.').next().unwrap().parse().unwrap();
        assert!(major >= 6, "{version}");
        assert_eq!(version.split('.').count(), 4, "{version}");
    }
}
