use std::{path::Path, process::{Command, Stdio}};

use clap::{Parser, Subcommand};
use anyhow::{Result, anyhow, bail};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: CLICommand
}

#[derive(Subcommand)]
enum CLICommand {
    Build,
    Disk,
    Run,
    Clean,
    BuildRun
}

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const CARGO: &str = env!("CARGO");
const KERNEL_ELF: &str = "kernel.elf";
const DISK_FILE: &str = "disk.img";
const LEMONFS_GUID: &str = "8CBD0D4E-FE62-49CC-A982-FBD6F69ED888";
const MOUNT_DIR: &str = "mount";
const COMPILED_DIR: &str = "compiled";
const OVMF: &str = "/usr/share/edk2/x64/OVMF.4m.fd";
const QEMU_RAM_MB: u64 = 128;
const CORES: u64 = 4;

const FILE_ARTIFACTS: [&str; 2] = [KERNEL_ELF, DISK_FILE];
const DIR_ARTIFACTS: [&str; 2] = [MOUNT_DIR, COMPILED_DIR];

fn main() -> Result<()> {
    let args = Cli::parse();
    
    let xtask_root = Path::new(ROOT);
    let proj_root = xtask_root.parent()
        .ok_or_else(|| anyhow!("Failed to get project root!"))?;

    match args.command {
        CLICommand::Build => build(proj_root),
        CLICommand::Clean => clean(proj_root),
        CLICommand::Disk => disk(proj_root),
        CLICommand::Run => run(proj_root),

        CLICommand::BuildRun => {
            build(proj_root)?;
            disk(proj_root)?;
            run(proj_root)?;
            Ok(())
        }
    }
}

fn build(proj_root: &Path) -> Result<()> {
    println!("Compiling the kernel...");

    // Get kernel path
    let kernel_path = proj_root.join("kernel");

    // Run cargo build --release
    let status = Command::new(CARGO)
        .current_dir(&kernel_path)
        .args(["build", "--release"])
        .status()?;

    // Handle failiure
    if !status.success() {
        eprintln!("Build failed!");
        eprintln!("Cargo returned error code {status}");
        eprintln!("Cargo path: {CARGO}");
        eprintln!("Kernel path: {}", kernel_path.display());

        std::process::exit(1);
    }

    // Copy executable to project root
    let kernel_executable = kernel_path.join("target")
        .join("x86_64-unknown-none")
        .join("release")
        .join("kernel");

    let destination = proj_root.join(KERNEL_ELF);
    let res = std::fs::copy(&kernel_executable, &destination);

    // Handle failiure
    if let Err(e) = res {
        println!("Failed to copy {} to {}!", kernel_executable.display(), destination.display());
        println!("The error was: {}", e);

        std::process::exit(1);
    }

    // Report success
    println!("Built the kernel!");
    println!("The executable can be found at {}", destination.display());

    Ok(())
}

fn clean(proj_root: &Path) -> Result<()> {
    // Get kernel path
    let kernel_path = proj_root.join("kernel");

    // Run cargo clean
    let status = Command::new(CARGO)
        .current_dir(&kernel_path)
        .arg("clean")
        .status()?;

    // Handle failiure
    if !status.success() {
        eprintln!("Clean failed!");
        eprintln!("Cargo returned error code {status}");
        eprintln!("Cargo path: {CARGO}");
        eprintln!("Kernel path: {}", kernel_path.display());
    }

    println!("Cleaned up kernel build artifacts!");

    // Clean up artifacts in project root
    for artifact in FILE_ARTIFACTS {
        let artifact_path = proj_root.join(artifact);

        if artifact_path.exists() {
            std::fs::remove_file(&artifact_path)?;
            println!("Removed file {}", artifact_path.display())
        }
    }

    for artifact in DIR_ARTIFACTS {
        let artifact_path = proj_root.join(artifact);

        if artifact_path.exists() {
            std::fs::remove_dir_all(&artifact_path)?;
            println!("Removed file {}", artifact_path.display())
        }
    }

    Ok(())
}

fn disk(proj_root: &Path) -> Result<()> {
    let disk_file = proj_root.join(DISK_FILE);
    let img = disk_file.to_string_lossy().into_owned();
    let kernel = proj_root.join(KERNEL_ELF).to_string_lossy().into_owned();

    // Create file
    sh("truncate", &["-s", "512M", &img], proj_root)?;

    // Create GPT + ESP
    sh("sgdisk", &["-o", &img], proj_root)?;
    sh("sgdisk", &["-n", "1:2048:+64M", "-t", "1:EF00", "-c", "1:ESP", &img], proj_root)?;

    let esp = format!("{img}@@{}", 2048 * 512);
    sh("mformat", &["-F", "-i", &esp, "-T", "131072"], proj_root)?;

    sh("mmd", &["-i", &esp, "::/EFI", "::/EFI/BOOT", "::/boot"], proj_root)?;

    sh("mcopy", &["-o", "-i", &esp, "limine-files/BOOTX64.EFI", "::/EFI/BOOT/BOOTX64.EFI"], proj_root)?;
    sh("mcopy", &["-o", "-i", &esp, "limine-files/limine.conf", "::/boot/limine.conf"], proj_root)?;
    sh("mcopy", &["-o", "-i", &esp, &kernel, "::/boot/kernel.elf"], proj_root)?;

    // Now LemonFS
    let part_w_guid = format!("2:{LEMONFS_GUID}");
    sh("sgdisk", &["-n", "2:0:0", "-t", &part_w_guid, "-c", "2:LemonFS", &img], proj_root)?;

    println!("Created partitions!");

    // Create mount point
    let mount_point = proj_root.join(MOUNT_DIR);
    if !mount_point.exists() { std::fs::create_dir(&mount_point)?; }

    // Mount
    let mut child = Command::new("lemonmnt")
        .current_dir(proj_root)
        .args([&img, MOUNT_DIR, "--format"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    compile(proj_root)?;

    let compile_path = proj_root.join("compiled");
    let bin_path = mount_point.join("bin");

    // Create /bin folder
    std::fs::create_dir(&bin_path)?;

    // Copy files
    for file in std::fs::read_dir(compile_path)? {
        let file = file?;
        let filename = file.file_name().to_string_lossy().into_owned();

        std::fs::copy(file.path(), bin_path.join(filename))?;
    }

    // Cleanup
    child.kill()?;
    sh("fusermount", &["-u", MOUNT_DIR], proj_root)?;

    println!("Made disk.img!");
    Ok(())
}

fn compile(proj_root: &Path) -> Result<()> {
    let bin_folder = proj_root.join("bins");
    let compiled_bins = proj_root.join("compiled");

    if !compiled_bins.exists() { std::fs::create_dir(&compiled_bins)?; }

    if !bin_folder.exists() { return Ok(()) } // Nothing to do
    let files = std::fs::read_dir(&bin_folder)?;

    for file in files {
        let file = file?;
        let name = file.file_name().to_string_lossy().into_owned();
        let path = file.path().to_string_lossy().into_owned();

        let output = path.clone() + ".o";

        if name.ends_with(".s") { // Assembly 
            let bin_name = name.strip_suffix(".s").unwrap();
            let bin_path = bin_folder.join(bin_name);
            let out_path = compiled_bins.join(bin_name.to_owned() + ".elf");

            sh("nasm", &["-f", "elf64", &path, "-o", &output], &bin_folder)?;
            sh("ld", &[&output, "-o", bin_name], &bin_folder)?;
            
            std::fs::remove_file(bin_folder.join(output))?;
            std::fs::rename(&bin_path, &out_path)?;

            println!("Compiled {name}!");
        }
    }

    Ok(())
}

fn run(proj_root: &Path) -> Result<()> {
    let qemu_args = [
        "-bios", OVMF, "-m", &format!("{QEMU_RAM_MB}M"), "-cpu", "max", "-machine", "q35",
        "-drive", &format!("if=none,id=sata0,file={DISK_FILE},format=raw"), "-device", "ich9-ahci,id=ahci",
        "-device", "ide-hd,bus=ahci.0,drive=sata0", "-smp", &format!("{CORES}")
    ];

    sh("qemu-system-x86_64", &qemu_args, proj_root)?;

    Ok(())
}

fn sh(program: &str, args: &[&str], cwd: &Path) -> Result<()> {
    let status = Command::new(program).args(args).current_dir(cwd).stdout(Stdio::null()).status()?;
    if !status.success() { bail!("{program} exited with {status}"); }
    Ok(())
}