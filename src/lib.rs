use nix::unistd;
use nix::sys::stat;
use std::collections::HashSet;
use std::error::Error;
use std::ffi::CString;
use std::fs;
use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::iter;
use std::os::unix::fs::MetadataExt;

struct Header {
    // Null-terminated character string
    name: [u8; 100],
    // File's permission bits, 0-padded octal with null terminator
    mode: [u8; 8],
    // Numerical ID of file's owner, 0-padded octal with null terminator
    uid: [u8; 8],
    // Numberical ID of file's group, 0-padded octal with null terminator
    gid: [u8; 8],
    // Size of file in bytes, 0-padded octal with null terminator
    size: [u8; 12],
    // File's modification time in Unix epoch time, 0-padded octal with null terminator
    mtime: [u8; 12],
    // Checksum (simple sum) of header bytes, 0-padded octal with null terminator
    checksum: [u8; 8],
    // File type (use constants defined below)
    typeflag: u8,
    // Unused for this project
    linkname: [u8; 100],
    // Indicates which tar standard we are using
    magic: [u8; 6],
    // Indicates which tar standard we are using, no null terminator
    version: [u8; 2],
    // Name of file's user, as null-terminated string
    uname: [u8; 32],
    // Name of file's group, as null-terminated string
    gname: [u8; 32],
    // Major device number, 0-padded octal with null terminator
    devmajor: [u8; 8],
    // Minor device number, 0-padded octal with null terminator
    devminor: [u8; 8],
    // String to prepend to 'name' field above, if file's name is longer than 100 bytes
    prefix: [u8; 155],
    // Padding to bring struct's size up to 512 bytes
    padding: [u8; 12],
}

const MAGIC: &str = "ustar";
const VERSION: &str = "00";
const REGTYPE: char = '0';
const FOOTER_SIZE: usize = 1024;

impl Header {
    fn from_file(file_name: &str) -> Result<Header, Box<dyn Error>> {
        let metadata = fs::metadata(file_name)?;

        // We'll assume file names are no longer than 99 ASCII characters for now
        let name = CString::new(file_name)?;
        let name = name.to_bytes_with_nul();
        let padded_name: Vec<u8> = name.iter().cloned().chain(iter::repeat(0u8)).take(100).collect();

        let mode = metadata.mode() & 0777;
        let mode = CString::new(format!("{:07o}", mode))?;
        let mode = mode.to_bytes_with_nul();

        let raw_uid = metadata.uid();
        let uid = CString::new(format!("{:07o}", raw_uid))?;
        let uid = uid.to_bytes_with_nul();
        let pwd = unistd::User::from_uid(unistd::Uid::from_raw(raw_uid))?;
        let pwd = pwd.ok_or("Failed to retrieve user's name with uid")?;
        let uname = CString::new(pwd.name)?;
        let uname = uname.to_bytes_with_nul();
        let padded_uname: Vec<u8>  = uname.iter().cloned().chain(iter::repeat(0u8)).take(32).collect();

        let raw_gid = metadata.gid();
        let gid = CString::new(format!("{:07o}", raw_gid))?;
        let gid = gid.to_bytes_with_nul();
        let grp = unistd::Group::from_gid(unistd::Gid::from_raw(raw_gid))?;
        let grp = grp.ok_or("Failed to retrieve group's name with gid")?;
        let gname = CString::new(grp.name)?;
        let gname = gname.to_bytes_with_nul();
        let padded_gname: Vec<u8> = gname.iter().cloned().chain(iter::repeat(0u8)).take(32).collect();

        let size = format!("{:011o}", metadata.len());
        let size = CString::new(size)?;
        let size = size.to_bytes_with_nul();

        let mtime = CString::new(format!("{:011o}", metadata.mtime()))?;
        let mtime = mtime.to_bytes_with_nul();
        let magic = CString::new(MAGIC)?;
        let magic = magic.to_bytes_with_nul();

        // Sadly, fs crate does not give us major/minor number extraction
        // So just use nix for this
        let dev = metadata.dev();
        let devmajor = CString::new(format!("{:07o}", stat::major(dev)))?;
        let devmajor = devmajor.to_bytes_with_nul();
        let devminor = CString::new(format!("{:07o}", stat::minor(dev)))?;
        let devminor = devminor.to_bytes_with_nul();

        let mut header = Header {
            name: padded_name.try_into().map_err(|_| "Failed to convert vector to slice (name)")?,
            mode: mode.try_into()?,
            uid: uid.try_into()?,
            gid: gid.try_into()?,
            size: size.try_into()?,
            mtime: mtime.try_into()?,
            checksum: [0; 8],
            typeflag: REGTYPE as u8,
            linkname: [0; 100],
            magic: magic.try_into()?,
            version: VERSION.as_bytes().try_into()?,
            uname: padded_uname.try_into().map_err(|_| "Failed to convert vector to slice (uname)")?,
            gname: padded_gname.try_into().map_err(|_| "Failed to convert vector to slice (gname)")?,
            devmajor: devmajor.try_into()?,
            devminor: devminor.try_into()?,
            prefix: [0; 155],
            padding: [0; 12],
        };

        header.fill_checksum()?;
        Ok(header)
    }

    fn as_bytes(&self) -> Vec<u8> {
        self.name.iter()
            .chain(self.mode.iter())
            .chain(self.uid.iter())
            .chain(self.gid.iter())
            .chain(self.size.iter())
            .chain(self.mtime.iter())
            .chain(self.checksum.iter())
            .chain(iter::once(&self.typeflag))
            .chain(self.linkname.iter())
            .chain(self.magic.iter())
            .chain(self.version.iter())
            .chain(self.uname.iter())
            .chain(self.gname.iter())
            .chain(self.devmajor.iter())
            .chain(self.devminor.iter())
            .chain(self.prefix.iter())
            .chain(self.padding.iter())
            .map(|x| x.clone())
            .collect()
    }

    fn from_bytes(bytes: &[u8]) -> Result<Header, Box<dyn Error>> {
        if bytes.len() != 512 {
            return Err(format!("Expected 512 bytes but got {}", bytes.len()).into());
        }

        let name = &bytes[0..100];
        let mode = &bytes[100..108];
        let uid = &bytes[108..116];
        let gid = &bytes[116..124];
        let size = &bytes[124..136];
        let mtime = &bytes[136..148];
        let checksum = &bytes[148..156];
        let typeflag = &bytes[156];
        let linkname = &bytes[157..257];
        let magic = &bytes[257..263];
        let version = &bytes[263..265];
        let uname = &bytes[265..297];
        let gname = &bytes[297..329];
        let devmajor = &bytes[329..337];
        let devminor = &bytes[337..345];
        let prefix = &bytes[345..500];
        let padding = &bytes[500..512];

        Ok(Header {
            name: name.try_into()?,
            mode: mode.try_into()?,
            uid: uid.try_into()?,
            gid: gid.try_into()?,
            size: size.try_into()?,
            mtime: mtime.try_into()?,
            checksum: checksum.try_into()?,
            typeflag: typeflag.clone(),
            linkname: linkname.try_into()?,
            magic: magic.try_into()?,
            version: version.try_into()?,
            uname: uname.try_into()?,
            gname: gname.try_into()?,
            devmajor: devmajor.try_into()?,
            devminor: devminor.try_into()?,
            prefix: prefix.try_into()?,
            padding: padding.try_into()?,
        })
    }

    fn fill_checksum(&mut self) -> Result<(), Box<dyn Error>> {
        self.checksum = [' ' as u8; 8];
        let checksum: u32 = self.as_bytes().into_iter().map(|x| x as u32).sum();
        let checksum = CString::new(format!("{:07o}", checksum))?;
        self.checksum = checksum.to_bytes_with_nul().try_into()?;
        Ok(())
    }
}

fn write_files_to_archive(archive: &mut fs::File, files: &[&str]) -> Result<(), Box<dyn Error>> {
    for file_name in files {
        let header = Header::from_file(file_name)?;
        archive.write_all(&header.as_bytes())?;
        let source_bytes = fs::read(file_name)?;
        archive.write_all(&source_bytes)?;
        let file_size = fs::metadata(file_name)?.len();
        let padding_size = 512 - (file_size % 512) as usize;
        if padding_size > 0 {
            let padding: Vec<u8> = iter::repeat(0).take(padding_size).collect();
            archive.write_all(&padding)?;
        }
    }

    let footer_blocks: Vec<u8> = iter::repeat(0).take(FOOTER_SIZE).collect();
    archive.write_all(&footer_blocks)?;

    Ok(())
}

pub fn create_archive(archive_name: &str, files: &[&str]) -> Result<(), Box<dyn Error>> {
    let mut archive = fs::File::create(archive_name)?;
    write_files_to_archive(&mut archive, files)?;
    Ok(())
}

pub fn append_to_archive(archive_name: &str, files: &[&str]) -> Result<(), Box<dyn Error>> {
    let mut archive = fs::OpenOptions::new().write(true).open(archive_name)?;
    archive.seek(io::SeekFrom::End(-1 * (FOOTER_SIZE as i64)))?;
    write_files_to_archive(&mut archive, files)?;
    Ok(())
}

pub fn get_archive_file_list(archive_name: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let mut archive = fs::File::open(archive_name)?;
    let mut header_bytes = [0; 512];

    let mut file_names: Vec<String> = Vec::new();
    loop {
        archive.read_exact(&mut header_bytes)?;
        if header_bytes.iter().map(|x| *x as u32).sum::<u32>() == 0 {
            // We've read in a footer block with all 0 bytes
            break;
        }
        let header = Header::from_bytes(&header_bytes)?;
        let name_bytes: Vec<u8> = header.name.iter().cloned().take_while(|x| *x != 0u8).collect();
        let name = String::from_utf8(name_bytes)?;
        file_names.push(name);

        let size_bytes: Vec<u8> = header.size.iter().cloned().take_while(|x| *x != 0u8).collect();
        let size_str = String::from_utf8(size_bytes)?;
        let size = u32::from_str_radix(&size_str, 8)?;
        let num_content_blocks = ((size  as f32) / 512.0).ceil() as i64;
        archive.seek(io::SeekFrom::Current(num_content_blocks * 512))?;
    }
    Ok(file_names)
}

pub fn update_archive(archive_name: &str, files: &[&str]) -> Result<(), Box<dyn Error>> {
    let existing_files = get_archive_file_list(archive_name)?;
    let existing_set: HashSet<&str> = existing_files.iter().map(|x| x.as_str()).collect();
    let to_add_set: HashSet<&str> = files.iter().cloned().collect();
    if !existing_set.is_superset(&to_add_set) {
        return Err("One or more of specified files not already present in archive".into());
    }

    append_to_archive(archive_name, files)
}

pub fn extract_from_archive(archive_name: &str) -> Result<(), Box<dyn Error>> {
    let mut archive = fs::File::open(archive_name)?;
    let mut header_bytes = [0; 512];

    loop {
        archive.read_exact(&mut header_bytes)?;
        if header_bytes.iter().map(|x| *x as u32).sum::<u32>() == 0 {
            // We've read in a footer block with all 0 bytes
            break;
        }
        let header = Header::from_bytes(&header_bytes)?;
        let name_bytes: Vec<u8> = header.name.iter().cloned().take_while(|x| *x != 0u8).collect();
        let name = String::from_utf8(name_bytes)?;
        let size_bytes: Vec<u8> = header.size.iter().cloned().take_while(|x| *x != 0u8).collect();
        let size = String::from_utf8(size_bytes)?;
        let size = u32::from_str_radix(&size, 8)?;

        let mut f = fs::File::create(name)?;
        let mut remaining_bytes = size as usize;
        while remaining_bytes > 0 {
            let chunk_size = if remaining_bytes >= 512 {
                512 as usize
            } else {
                remaining_bytes as usize
            };
            let mut buf = vec![0u8; chunk_size];
            archive.read_exact(&mut buf)?;
            f.write_all(&buf)?;
            remaining_bytes -= chunk_size;
        }

        let num_padding_bytes = 512 - (size % 512);
        archive.seek(io::SeekFrom::Current(num_padding_bytes as i64))?;
    }

    Ok(())
}
