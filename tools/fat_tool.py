#!/usr/bin/env python3
import argparse
import shutil
import struct
import subprocess
import sys

ESP_GUID = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B"
MBR_SIGNATURE = 0xAA55
GPT_SIGNATURE = b"EFI PART"


class FatToolError(Exception):
    pass


def read_sectors(f, count, sector):
    f.seek(sector * 512)
    return f.read(count * 512)


def parse_guid_le(b):
    d1, d2, d3 = struct.unpack_from("<IHH", b, 0)
    d4 = b[8:10].hex()
    d5 = b[10:16].hex()
    return f"{d1:08x}-{d2:04x}-{d3:04x}-{d4}-{d5}".upper()


def gpt_partitions(path):
    try:
        f = open(path, "rb")
    except OSError as e:
        raise FatToolError(f"cannot open image '{path}': {e.strerror}") from e
    with f:
        mbr = read_sectors(f, 1, 0)
        if len(mbr) < 512 or struct.unpack_from("<H", mbr, 510)[0] != MBR_SIGNATURE:
            return None
        hdr = read_sectors(f, 1, 1)
        if len(hdr) < 512 or hdr[0:8] != GPT_SIGNATURE:
            return None
        entries_lba = struct.unpack_from("<Q", hdr, 72)[0]
        num_entries = struct.unpack_from("<I", hdr, 80)[0]
        entry_size = struct.unpack_from("<I", hdr, 84)[0]
        count = num_entries * entry_size
        data = b""
        sector = entries_lba
        while len(data) < count:
            chunk = read_sectors(f, 1, sector)
            if not chunk:
                break
            data += chunk
            sector += 1
        parts = []
        for i in range(num_entries):
            e = data[i * entry_size:(i + 1) * entry_size]
            if not e or e[0:16] == b"\x00" * 16:
                continue
            typ = parse_guid_le(e[0:16])
            first_lba = struct.unpack_from("<Q", e, 32)[0]
            last_lba = struct.unpack_from("<Q", e, 40)[0]
            name = e[56:128].decode("utf-16-le", errors="replace").rstrip("\x00")
            parts.append({
                "index": i + 1,
                "type_guid": typ,
                "first_lba": first_lba,
                "last_lba": last_lba,
                "name": name,
            })
        return parts


def looks_like_raw_fat(path):
    try:
        with open(path, "rb") as f:
            b = read_sectors(f, 1, 0)
    except OSError:
        return False
    if len(b) < 512 or struct.unpack_from("<H", b, 510)[0] != MBR_SIGNATURE:
        return False
    bps = struct.unpack_from("<H", b, 11)[0]
    return bps in (512, 1024, 2048, 4096)


def find_partition(path, partition_index=None, offset=None):
    if offset is not None:
        return None, None
    parts = gpt_partitions(path)
    if parts is None:
        return None, None
    if partition_index is not None:
        for p in parts:
            if p["index"] == partition_index:
                return p, parts
        return None, parts
    for p in parts:
        if p["type_guid"] == ESP_GUID:
            return p, parts
    return None, parts


def mt_extra(image, part, offset):
    if offset is not None:
        return f"{image}@@{offset}"
    if part is None:
        return image
    return f"{image}@@{part['first_lba'] * 512}"


def run_mtool(tool, args):
    exe = shutil.which(tool)
    if exe is None:
        raise FatToolError(f"'{tool}' not found — install mtools")
    proc = subprocess.run([exe, *args], capture_output=True, text=True)
    return proc


def partition_formatted(path, part, offset):
    if part is None and offset is None:
        return False
    start = offset if offset is not None else part["first_lba"] * 512
    try:
        with open(path, "rb") as f:
            f.seek(start)
            b = f.read(512)
    except OSError:
        return False
    if len(b) != 512:
        return False
    if struct.unpack_from("<H", b, 510)[0] != MBR_SIGNATURE:
        return False
    bps = struct.unpack_from("<H", b, 11)[0]
    if bps not in (512, 1024, 2048, 4096):
        return False
    spc = b[13]
    if spc == 0 or (spc & (spc - 1)) != 0:
        return False
    total16 = struct.unpack_from("<H", b, 19)[0]
    total32 = struct.unpack_from("<I", b, 32)[0]
    spf16 = struct.unpack_from("<H", b, 22)[0]
    spf32 = struct.unpack_from("<I", b, 36)[0]
    return (total16 or total32) != 0 and (spf16 or spf32) != 0


def get_image(args):
    if args.offset is not None:
        return None, None
    part, parts = find_partition(args.image, args.partition, None)
    if parts is not None:
        if part is None:
            raise FatToolError(
                f"no FAT/ESP partition found in {args.image} "
                "(try --partition N)")
        return part, parts
    if looks_like_raw_fat(args.image):
        return None, None
    raise FatToolError(
        f"{args.image}: not a GPT partition table or raw FAT image "
        "(give --partition or --offset)")


def mt_image(image, part, offset):
    return mt_extra(image, part, offset)


def target_path(path):
    if path in (None, "", "/"):
        return "::"
    return "::/" + path.lstrip("/")


def cmd_info(args):
    part, parts = get_image(args)
    if parts:
        print("GPT partition table:")
        for p in parts:
            start = p["first_lba"] * 512
            end = (p["last_lba"] + 1) * 512
            size_mib = (end - start) / (1024 * 1024)
            star = "*" if p is part else " "
            print(f"{star} {p['index']:2d}  {p['type_guid']}  "
                  f"{p['name']:<12} {p['first_lba']:>10d}..{p['last_lba']:<10d} "
                  f"{size_mib:8.1f} MiB")
    else:
        print("no GPT table found; treating image as raw FAT")
    if part:
        print(f"FAT partition: {part['index']} '{part['name']}' "
              f"at byte offset {part['first_lba'] * 512}")
    elif args.offset is not None:
        print(f"FAT partition: explicit byte offset {args.offset}")
    else:
        print("FAT partition: starts at byte offset 0")


def cmd_ls(args):
    part, parts = get_image(args)
    bits = ["-i", mt_image(args.image, part, args.offset)]
    if args.recursive:
        bits.append("-/")
    if args.all:
        bits.append("-a")
    bits.append(target_path(args.path))
    proc = run_mtool("mdir", bits)
    sys.stdout.write(proc.stdout)
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        sys.exit(proc.returncode)


def ensure_dir(image, part, offset, far_dir):
    if far_dir in ("", "/"):
        return
    parts = far_dir.strip("/").split("/")
    acc = ""
    for comp in parts:
        acc = acc + "/" + comp
        proc = run_mtool(
            "mdir", ["-i", mt_image(image, part, offset), target_path(acc)])
        if proc.returncode != 0:
            mk = run_mtool(
                "mmd", ["-i", mt_image(image, part, offset), target_path(acc)])
            if mk.returncode != 0:
                sys.stderr.write(mk.stderr)
                sys.exit(mk.returncode)


def cmd_cat(args):
    part, parts = get_image(args)
    img = mt_image(args.image, part, args.offset)
    if args.output:
        proc = run_mtool("mcopy", ["-o", "-i", img, target_path(args.path), args.output])
        if proc.returncode != 0:
            sys.stderr.write(proc.stderr)
            sys.exit(proc.returncode)
        return
    proc = run_mtool("mcopy", ["-o", "-i", img, target_path(args.path), "-"])
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        sys.exit(proc.returncode)
    sys.stdout.write(proc.stdout)
    sys.stdout.flush()


def cmd_write(args):
    part, parts = get_image(args)
    far = target_path(args.fat_path)
    parent = args.fat_path.rsplit("/", 1)[0] if "/" in args.fat_path else "/"
    ensure_dir(args.image, part, args.offset, parent)
    proc = run_mtool(
        "mcopy",
        ["-o", "-i", mt_image(args.image, part, args.offset), args.host_path, far])
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        sys.exit(proc.returncode)


def cmd_mkdir(args):
    part, parts = get_image(args)
    img = mt_image(args.image, part, args.offset)
    for path in args.paths:
        parent = path.rsplit("/", 1)[0] if "/" in path else "/"
        ensure_dir(args.image, part, args.offset, parent)
        exists = run_mtool("mdir", ["-i", img, target_path(path)]).returncode == 0
        if exists:
            continue
        proc = run_mtool("mmd", ["-i", img, target_path(path)])
        sys.stderr.write(proc.stderr)
        if proc.returncode != 0:
            sys.exit(proc.returncode)


def cmd_rm(args):
    part, parts = get_image(args)
    for path in args.paths:
        proc = run_mtool(
            "mdel", ["-i", mt_image(args.image, part, args.offset), target_path(path)])
        sys.stderr.write(proc.stderr)
        if proc.returncode != 0:
            sys.exit(proc.returncode)


def cmd_format(args):
    part, parts = get_image(args)
    bits = ["-F", "-i", mt_image(args.image, part, args.offset)]
    if part is not None:
        total = part["last_lba"] - part["first_lba"] + 1
        bits += ["-T", str(total)]
    elif args.sectors is not None:
        bits += ["-T", str(args.sectors)]
    proc = run_mtool("mformat", bits)
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        sys.exit(proc.returncode)


def cmd_is_formatted(args):
    part, parts = get_image(args)
    if partition_formatted(args.image, part, args.offset):
        sys.stderr.write("formatted\n")
        return
    sys.stderr.write("not formatted\n")
    sys.exit(1)


def build_parser():
    ap = argparse.ArgumentParser(prog="fat_tool", description=(
        "Access and manipulate files on the FAT (ESP) partition of a "
        "GPT-formatted disk image by driving mtools."))
    ap.add_argument("--image", default="disk.img",
                    help="disk image (default: disk.img)")
    ap.add_argument("--partition", type=int, default=None,
                    help="partition number to use (default: first FAT/ESP)")
    ap.add_argument("--offset", type=int, default=None,
                    help="byte offset of FAT partition (skips GPT parse)")
    sub = ap.add_subparsers(dest="command", required=True)

    p = sub.add_parser("info", help="show GPT layout and FAT partition location")
    p.set_defaults(func=cmd_info)

    p = sub.add_parser("ls", help="list files (mdir)")
    p.add_argument("path", nargs="?", default="/")
    p.add_argument("-r", "--recursive", action="store_true", help="recurse into dirs")
    p.add_argument("-a", "--all", action="store_true", help="include hidden files")
    p.set_defaults(func=cmd_ls)

    p = sub.add_parser("cat", help="extract a file from the image (mcopy)")
    p.add_argument("path", help="file inside the image, e.g. /boot/kernel.elf")
    p.add_argument("output", nargs="?", default=None,
                   help="host output path (default: stdout)")
    p.set_defaults(func=cmd_cat)

    p = sub.add_parser("write", help="copy a host file into the image (mcopy)")
    p.add_argument("host_path", help="host source file")
    p.add_argument("fat_path", help="destination inside the image, e.g. /boot/kernel.elf")
    p.set_defaults(func=cmd_write)

    p = sub.add_parser("mkdir", help="create directories in the image (mmd)")
    p.add_argument("paths", nargs="+", help="directories to create, e.g. /EFI/BOOT")
    p.set_defaults(func=cmd_mkdir)

    p = sub.add_parser("rm", help="delete files in the image (mdel)")
    p.add_argument("paths", nargs="+", help="files to delete")
    p.set_defaults(func=cmd_rm)

    p = sub.add_parser("format", help="format the FAT partition (mformat -F)")
    p.add_argument("--sectors", type=int, default=None,
                   help="partition size in sectors (used with --offset)")
    p.set_defaults(func=cmd_format)

    p = sub.add_parser("is-formatted", help=(
        "exit 0 if the FAT partition already holds a FAT filesystem, else 1"))
    p.set_defaults(func=cmd_is_formatted)

    return ap


def main():
    parser = build_parser()
    args = parser.parse_args()
    try:
        args.func(args)
    except FatToolError as e:
        sys.stderr.write(f"fat_tool: error: {e}\n")
        sys.exit(1)


if __name__ == "__main__":
    main()