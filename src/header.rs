use std::ffi::{CStr, CString, OsStr};
use std::io::{Cursor, Read, Result as IoResult, Write};
use std::os::unix::prelude::{MetadataExt, OsStrExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bincode::Options;
use nix::sys::stat;
use nix::unistd::{Group, User};

use crate::REGTYPE;

#[derive(Debug, Serialize, Deserialize)]
pub struct Header {
  /// Null-terminated character string
  pub name: PathBuf,

  /// File's permission bits, 0-padded octal with null terminator
  pub mode: u32,

  /// Numerical ID of file's owner, 0-padded octal with null terminator
  pub uid: u32,

  /// Numberical ID of file's group, 0-padded octal with null terminator
  pub gid: u32,

  /// Size of file in bytes, 0-padded octal with null terminator
  pub size: u64,

  /// File's modification time in Unix epoch time, 0-padded octal with null
  /// terminator
  pub mtime: SystemTime,

  /// File type (use constants defined below)
  pub typeflag: u8,

  /// Unused for this project
  pub linkname: Option<PathBuf>,

  pub magic: [u8; 6],
  pub version: [u8; 2],

  /// Name of file's user, as null-terminated string
  pub uname: String,

  /// Name of file's group, as null-terminated string
  pub gname: String,

  /// Major device number, 0-padded octal with null terminator
  pub devmajor: u64,

  /// Minor device number, 0-padded octal with null terminator
  pub devminor: u64,

  /// String to prepend to 'name' field above, if file's name is longer than
  /// 100 bytes
  pub prefix: String,
}

impl Header {
  /// Create a Header for a given file path
  pub fn new(path: impl AsRef<Path>) -> Result<Self> {
    let path = path.as_ref();
    let meta = path.metadata()?;

    let uid = meta.uid();
    let gid = meta.gid();

    let user = User::from_uid(uid.into())?
      .ok_or_else(|| anyhow!("no user with id {uid}"))?;
    let group = Group::from_gid(gid.into())?
      .ok_or_else(|| anyhow!("no group with id {gid}"))?;

    let dev = meta.dev();

    Ok(Header {
      name: path.to_path_buf(),
      mode: meta.mode(),
      uid,
      gid,
      size: meta.size(),
      mtime: meta.modified()?,
      typeflag: REGTYPE,
      linkname: None,
      magic: *b"ustar\0",
      version: *b"00",
      uname: user.name,
      gname: group.name,
      devmajor: stat::major(dev),
      devminor: stat::minor(dev),
      prefix: String::new(),
      // atime: UNIX_EPOCH + Duration::from_secs(meta.atime() as u64),
      // ctime: UNIX_EPOCH + Duration::from_secs(meta.ctime() as u64),
    })
  }

  /// Read a Header using any Read. If there's no more headers, returns None.
  pub fn read(mut r: impl Read) -> Result<Option<Self>> {
    // Read exactly 512 bytes for the header
    let mut buf = [0; 512];
    r.read_exact(&mut buf)?;

    // Fast zero check https://stackoverflow.com/a/65376133
    let is_zero = {
      let (prefix, aligned, suffix) = unsafe { buf.align_to::<u128>() };
      prefix.iter().all(|&x| x == 0)
        && suffix.iter().all(|&x| x == 0)
        && aligned.iter().all(|&x| x == 0)
    };
    if is_zero {
      return Ok(None);
    }

    // Parse the header
    let reader = HeaderReader::new(&buf);
    let header = reader.read()?;
    Ok(Some(header))
  }

  pub fn write(&self, w: impl Write) -> Result<()> {
    let writer = HeaderWriter {
      header: self,
      w,
      written: 0,
    };
    writer.write(true)
  }

  pub fn compute_checksum(&self) -> Result<u32> {
    let vec = Vec::<u8>::new();
    let mut cursor = Cursor::new(vec);

    let writer = HeaderWriter {
      header: self,
      w: &mut cursor,
      written: 0,
    };
    writer.write(false)?;

    let vec = cursor.into_inner();
    let sum = vec.iter().map(|u| *u as u32).sum();
    Ok(sum)
  }
}

/// Helper struct for reading headers
pub struct HeaderReader<'a> {
  pos: usize,
  bytes: &'a [u8],
}

impl<'a> HeaderReader<'a> {
  pub fn new(buf: &'a [u8]) -> Self {
    HeaderReader { pos: 0, bytes: buf }
  }

  pub fn read(mut self) -> Result<Header> {
    let bin_reader = bincode::DefaultOptions::default().with_fixint_encoding();

    let name = self.read_path(100)?;
    let mode = self.read_octal_32(8).context("could not read mode")?;
    let uid = self.read_octal_32(8).context("could not read uid")?;
    let gid = self.read_octal_32(8).context("could not read gid")?;
    let size = self.read_64(12, 8).context("could not read size")?;

    let mtime = self.read_time(12).context("could not read mtime")?;
    let recorded_checksum = self.read_octal_32(8)?;

    let typeflag = {
      let vec = self.read_fixed(1)?;
      vec[0]
    };

    let linkname = Some(self.read_path(100)?);
    let magic = self.read_fixed(6)?.try_into()?;
    let version = self.read_fixed(2)?.try_into()?;

    let uname = self.read_string(32).context("could not read uname")?;
    let gname = self.read_string(32).context("could not read gname")?;

    let devmajor = {
      let s = self.read_fixed(8).context("could not read devmajor")?;
      bin_reader.deserialize(s)?
    };
    let devminor = {
      let s = self.read_fixed(8).context("could not read devminor")?;
      bin_reader.deserialize(s)?
    };

    let prefix = self.read_string(155)?;

    let header = Header {
      name,
      mode,
      uid,
      gid,
      size,
      mtime,
      typeflag,
      linkname,
      magic,
      version,
      uname,
      gname,
      devmajor,
      devminor,
      prefix,
    };

    let expected_checksum = header.compute_checksum()?;

    ensure!(
      expected_checksum == recorded_checksum,
      "Checksums do not match, expected: {}, recorded: {}",
      expected_checksum,
      recorded_checksum
    );

    Ok(header)
  }

  #[inline]
  fn read_time(&mut self, len: usize) -> Result<SystemTime> {
    let int = self.read_64(len, 8)?;
    let duration = Duration::from_secs(int);
    Ok(UNIX_EPOCH + duration)
  }

  #[inline]
  fn read_64(&mut self, len: usize, radix: u32) -> Result<u64> {
    let string = self.read_string(len)?;
    let int = u64::from_str_radix(&string, radix)?;
    Ok(int)
  }

  #[inline]
  fn read_octal_32(&mut self, len: usize) -> Result<u32> {
    let string = self.read_string(len)?;
    let int = u32::from_str_radix(&string, 8)?;
    Ok(int)
  }

  /// Turn a null-terminated string into a normal one
  #[inline]
  fn read_string(&mut self, len: usize) -> Result<String> {
    let cstr = self.read_cstring(len)?;
    let s = std::str::from_utf8(cstr.to_bytes())?;
    Ok(s.to_owned())
  }

  fn read_path(&mut self, len: usize) -> Result<PathBuf> {
    let cstr = self.read_cstring(len).context("could not read cstr")?;
    let osstr = OsStr::from_bytes(cstr.to_bytes());
    let path = PathBuf::from(osstr);
    Ok(path)
  }

  #[inline]
  fn read_cstring(&mut self, len: usize) -> Result<&CStr> {
    let string = self
      .read_fixed(len)
      .with_context(|| format!("could not read {len} bytes"))?;
    let idx_zero = match string.iter().position(|x| *x == 0) {
      Some(v) => v,
      None => bail!("not null-terminated"),
    };

    let new_slice = &string[..idx_zero + 1];
    let cstr =
      CStr::from_bytes_with_nul(new_slice).context("could not create cstr")?;
    Ok(cstr)
  }

  #[inline]
  fn read_fixed(&mut self, len: usize) -> Result<&[u8]> {
    let start = self.pos;
    let end = self.pos + len;

    if end > self.bytes.len() {
      bail!("reached end of file");
    }

    self.pos += len;
    Ok(&self.bytes[start..end])
  }
}

/// Helper struct for writing headers
pub struct HeaderWriter<'a, W: Write> {
  header: &'a Header,
  w: W,
  written: usize,
}

impl<'a, W: Write> HeaderWriter<'a, W> {
  pub fn write(mut self, write_checksum: bool) -> Result<()> {
    self
      .write_path(&self.header.name, 100)
      .context("could not write name")?;

    self
      .write_octal_32(self.header.mode, 8)
      .context("could not write mode")?;
    self
      .write_octal_32(self.header.uid, 8)
      .context("could not write uid")?;
    self
      .write_octal_32(self.header.gid, 8)
      .context("could not write gid")?;
    self
      .write_octal_64(self.header.size, 12)
      .context("could not write size")?;

    self.write_time(self.header.mtime, 12)?;

    if write_checksum {
      let checksum = self.header.compute_checksum()?;
      self.write_octal_32(checksum, 8)?;
    } else {
      let checksum = [32; 8];
      self.inner_write(&checksum)?;
    }

    let typeflag: [u8; 1] = [self.header.typeflag];
    self.inner_write(&typeflag)?;

    let linkname = [0; 100];
    self.inner_write(&linkname)?;
    // let linkname = Some(self.read_path(100)?);

    self.inner_write(&self.header.magic)?;
    self.inner_write(&self.header.version)?;

    self.write_cstring(&self.header.uname, 32)?;
    self.write_cstring(&self.header.gname, 32)?;

    let bin_writer = bincode::DefaultOptions::default().with_fixint_encoding();

    let devmajor = bin_writer.serialize(&self.header.devmajor)?;
    ensure!(devmajor.len() == 8);
    self.inner_write(&devmajor)?;

    let devminor = bin_writer.serialize(&self.header.devminor)?;
    ensure!(devmajor.len() == 8);
    self.inner_write(&devminor)?;

    self.write_cstring(&self.header.prefix, 155)?;

    // 12 bytes off, so just write some padding
    let padding = [0; 12];
    self.inner_write(&padding)?;

    Ok(())
  }

  #[inline]
  fn inner_write(&mut self, buf: &[u8]) -> IoResult<usize> {
    /* println!(
      "[{} .. {}] writing {} bytes: {:?}",
      self.written,
      self.written + buf.len(),
      buf.len(),
      std::str::from_utf8(buf)
    ); */
    self.written += buf.len();

    self.w.write(buf)
  }

  #[inline]
  fn write_path(&mut self, path: impl AsRef<Path>, len: usize) -> Result<()> {
    let path = path.as_ref();
    let osstr = path.as_os_str();
    let bytes = osstr.as_bytes();
    self.write_cstring(bytes, len)
  }

  #[inline]
  fn write_octal_32(&mut self, num: u32, len: usize) -> Result<()> {
    let s = format!("{:0width$o}", num, width = len - 1);
    ensure!(s.len() == len - 1);
    self.write_cstring(s, len)
  }

  #[inline]
  fn write_octal_64(&mut self, num: u64, len: usize) -> Result<()> {
    let s = format!("{:0width$o}", num, width = len - 1);
    ensure!(s.len() == len - 1);
    self.write_cstring(s, len)
  }

  #[inline]
  fn write_time(&mut self, time: SystemTime, len: usize) -> Result<()> {
    let elapsed = time.duration_since(UNIX_EPOCH)?;
    let secs = elapsed.as_secs();
    self.write_octal_64(secs, len)
  }

  #[inline]
  fn write_cstring(
    &mut self,
    string: impl AsRef<[u8]>,
    len: usize,
  ) -> Result<()> {
    let string = string.as_ref();
    let cstr = CString::new(string)?;
    let bytes = cstr.as_bytes();
    self.inner_write(bytes)?;

    let zeros = vec![0; len - bytes.len()];
    self.inner_write(&zeros)?;

    Ok(())
  }
}
