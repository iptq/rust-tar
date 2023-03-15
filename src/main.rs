#[macro_use]
extern crate anyhow;

use std::env;

use anyhow::Result;

fn main() -> Result<()> {
  let args: Vec<String> = env::args().collect();
  if args.len() < 4 {
    bail!("Usage: {} -[c|a|t|x] -f <archive_name> <files>", &args[0]);
  }

  let archive_name = &args[3];
  let file_names: Vec<&str> = args.iter().map(|x| x.as_str()).skip(4).collect();

  match args[1].as_str() {
    "-c" => minitar::create_archive(archive_name, &file_names),
    "-a" => minitar::append_to_archive(archive_name, &file_names),
    "-t" => {
      let file_names = minitar::get_archive_file_list(archive_name)?;
      for name in file_names {
        println!("{}", name);
      }
      return Ok(());
    }
    "-u" => minitar::update_archive(archive_name, &file_names),
    "-x" => minitar::extract_from_archive(archive_name),
    _ => {
      bail!("Unknown operation {}", &args[1]);
    }
  }?;

  Ok(())
}
