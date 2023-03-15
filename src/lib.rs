#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate serde;

pub mod header;

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::iter;

use anyhow::{Context, Result};

use crate::header::Header;

const REGTYPE: u8 = b'0';
const FOOTER_SIZE: usize = 1024;

fn write_files_to_archive(
  mut archive: &mut File,
  files: &[&str],
) -> Result<()> {
  for path in files {
    let header: Header = Header::new(path)?;
    header.write(&mut archive)?;

    let source_bytes = fs::read(path)?;
    archive.write_all(&source_bytes)?;

    let file_size = header.size;
    let padding_size = 512 - (file_size % 512) as usize;
    if padding_size > 0 {
      let padding: Vec<u8> = iter::repeat(0).take(padding_size).collect();
      archive.write_all(&padding)?;
    }
  }

  let footer_blocks = vec![0; FOOTER_SIZE];
  archive.write_all(&footer_blocks)?;

  Ok(())
}

pub fn create_archive(archive_name: &str, files: &[&str]) -> Result<()> {
  let mut archive = File::create(archive_name)?;
  write_files_to_archive(&mut archive, files)?;
  Ok(())
}

pub fn append_to_archive(archive_name: &str, files: &[&str]) -> Result<()> {
  let mut archive = OpenOptions::new().append(true).open(archive_name)?;
  archive.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;
  write_files_to_archive(&mut archive, files)?;
  Ok(())
}

pub fn get_archive_file_list(archive_name: &str) -> Result<Vec<String>> {
  let mut archive = File::open(archive_name)?;
  let mut file_names = Vec::new();

  loop {
    let header_opt =
      Header::read(&mut archive).context("could not parse header")?;

    let header: Header = match header_opt {
      Some(v) => v,
      None => break,
    };

    file_names.push(header.name.display().to_string());

    let num_content_blocks = (header.size as i64 + 511) / 512;
    let content_bytes = num_content_blocks * 512;
    archive.seek(SeekFrom::Current(content_bytes))?;
  }

  Ok(file_names)
}

pub fn update_archive(archive_name: &str, files: &[&str]) -> Result<()> {
  let existing_files = get_archive_file_list(archive_name)?;
  let existing_set: HashSet<&str> =
    existing_files.iter().map(|x| x.as_str()).collect();
  let to_add_set: HashSet<&str> = files.iter().cloned().collect();
  if !existing_set.is_superset(&to_add_set) {
    bail!("One or more of specified files not already present in archive");
  }

  append_to_archive(archive_name, files)
}

pub fn extract_from_archive(archive_name: &str) -> Result<()> {
  let mut archive = fs::File::open(archive_name)?;

  loop {
    let header = match Header::read(&mut archive)? {
      Some(v) => v,
      None => break,
    };

    let mut f = File::create(header.name)?;
    let mut remaining_bytes = header.size as usize;
    while remaining_bytes > 0 {
      let chunk_size = if remaining_bytes >= 512 {
        512_usize
      } else {
        remaining_bytes
      };
      let mut buf = vec![0u8; chunk_size];
      archive.read_exact(&mut buf)?;
      f.write_all(&buf)?;

      remaining_bytes -= chunk_size;
    }

    let num_padding_bytes = 512 - (header.size % 512);
    archive.seek(SeekFrom::Current(num_padding_bytes as i64))?;
  }

  Ok(())
}
