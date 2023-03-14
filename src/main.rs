use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        println!("Usage: {} -[c|a|t|x] -f <archive_name> <files>", &args[0]);
        return ExitCode::from(1);
    }

    let archive_name = &args[3];
    let file_names: Vec<&str> = args.iter().map(|x| x.as_str()).skip(4).collect();
    let result = match args[1].as_str() {
        "-c" => minitar::create_archive(archive_name, &file_names),
        "-a" => minitar::append_to_archive(archive_name, &file_names),
        "-t" => minitar::get_archive_file_list(archive_name).map(|file_names| {
            for name in file_names {
                println!("{}", name);
            }
        }),
        "-u" => minitar::update_archive(archive_name, &file_names),
        "-x" => minitar::extract_from_archive(archive_name),
        _ => {
            eprintln!("Unknown operation {}", &args[1]);
            return ExitCode::from(1);
        }
    };

    match result {
        Ok(_) => ExitCode::from(0),
        Err(e) => {
            eprintln!("Archive operation failed: {e}");
            ExitCode::from(1)
        }
    }
}
