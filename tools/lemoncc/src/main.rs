use std::fs;
use std::path::Path;

mod lemonfs;

fn print_usage() {
    eprintln!("Usage: lemoncc <input.elf> [-o <output_path>] [-d <disk_image>]");
    eprintln!();
    eprintln!("  <input.elf>     Pre-compiled ELF binary to inject (positional)");
    eprintln!("  -o <path>       Destination path in LemonFS (default: /<stem>.elf)");
    eprintln!("  -d <img>        Disk image file (default: drive.img)");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  lemoncc bin.elf");
    eprintln!("  lemoncc bin.elf -o /bin.elf");
    eprintln!("  lemoncc bin.elf -o /bin.elf -d nvme.img");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut input_file: Option<String> = None;
    let mut output_path: Option<String> = None;
    let mut img_path = "drive.img".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: -o requires an argument");
                    print_usage();
                    std::process::exit(1);
                }
                output_path = Some(args[i].clone());
            }
            "-d" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: -d requires an argument");
                    print_usage();
                    std::process::exit(1);
                }
                img_path = args[i].clone();
            }
            "-h" | "--help" => {
                print_usage();
                return;
            }
            _ => {
                if input_file.is_none() {
                    input_file = Some(args[i].clone());
                } else {
                    eprintln!("Error: unexpected argument '{}'", args[i]);
                    print_usage();
                    std::process::exit(1);
                }
            }
        }
        i += 1;
    }

    let input_file = input_file.unwrap_or_else(|| {
        eprintln!("Error: no input file specified");
        print_usage();
        std::process::exit(1);
    });

    if !Path::new(&input_file).exists() {
        eprintln!("Error: input file '{}' not found", input_file);
        std::process::exit(1);
    }

    let output_path = output_path.unwrap_or_else(|| {
        let stem = Path::new(&input_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        format!("/{}.elf", stem)
    });

    let binary = fs::read(&input_file).unwrap_or_else(|e| {
        eprintln!("Error: failed to read '{}': {}", input_file, e);
        std::process::exit(1);
    });

    let mut fs = lemonfs::LemonFS::open(&img_path).unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    });

    fs.write_file(&output_path, &binary).unwrap_or_else(|e| {
        eprintln!("Error: failed to write to '{}': {}", output_path, e);
        std::process::exit(1);
    });

    println!(
        "Injected {} -> {} ({} bytes written to {})",
        input_file, output_path, binary.len(), img_path
    );
}
