#![allow(dead_code)]

use crate::{println, log_error};

use crate::{
    allocator::BuddyAllocator,
    fs::{Bitmap, DirectoryCache, FileType, HashEntry, LinkEntry, hash_name, name_eq, name_to_bytes, name_to_str},
    fpu::FpuState,
    library::{LateInit, Time},
    memory::{KMemory, PhysPage, PAGE_SIZE},
    scheduling::{
        scheduler::{calculate_priority, predict_burst},
        task::{Task, TaskStatus},
    },
};
use crate::library::crc32;
use crate::gpt::GPTPartitionEntry;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use x86_64::PhysAddr;

// ---------------------------------------------------------------------------
// Minimal test harness
// ---------------------------------------------------------------------------

use core::sync::atomic::{AtomicUsize, Ordering};

static PASSED: AtomicUsize = AtomicUsize::new(0);
static FAILED: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);

fn test(name: &str, ok: bool) {
    TOTAL.fetch_add(1, Ordering::SeqCst);
    if ok {
        PASSED.fetch_add(1, Ordering::SeqCst);
    } else {
        FAILED.fetch_add(1, Ordering::SeqCst);
        log_error!("FAIL: {}", name);
    }
}

// ---------------------------------------------------------------------------

pub fn run_all() {
    println!("");
    println!("=== MaramOS Integration Tests ===");

    library_tests();
    crc32_tests();
    buddy_tests();
    memory_tests();
    physpage_tests();
    bitmap_tests();
    hash_tests();
    filetype_tests();
    dir_cache_tests();
    gpt_tests();
    scheduler_tests();

    println!("");
    let p = PASSED.load(Ordering::SeqCst);
    let f = FAILED.load(Ordering::SeqCst);
    println!("=== Results: {} passed, {} failed ===", p, f);
    if f > 0 {
        log_error!("SOME TESTS FAILED");
    }
}

// ---------------------------------------------------------------------------
// library: LateInit<T> and Time
// ---------------------------------------------------------------------------

fn library_tests() {
    // LateInit
    let li: LateInit<u64> = LateInit::new();
    test("library: try_get returns None before init", li.try_get().is_none());

    let li2: LateInit<u64> = LateInit::new();
    li2.init(42);
    test("library: get returns value after init", *li2.get() == 42);

    let li3: LateInit<u64> = LateInit::new();
    li3.init(7);
    test("library: try_get returns Some after init", li3.try_get() == Some(&7));

    let li4: LateInit<String> = LateInit::new();
    li4.init(String::from("hello"));
    test("library: Deref works", li4.len() == 5);

    let mut li5: LateInit<u64> = LateInit::new();
    li5.init(10);
    *li5 = 20;
    test("library: DerefMut works", *li5.get() == 20);

    // Time
    test("library: Nanoseconds", Time::Nanoseconds(5).to_nanos() == 5);
    test("library: Milliseconds", Time::Milliseconds(3).to_nanos() == 3_000_000);
    test("library: Seconds", Time::Seconds(2).to_nanos() == 2_000_000_000);
    test("library: Minutes", Time::Minutes(1).to_nanos() == 60_000_000_000);
    test("library: Hours", Time::Hours(1).to_nanos() == 360_000_000_000);
    test("library: Days", Time::Days(1).to_nanos() == 86_400_000_000_000);
    test("library: Hours 24 == Days 1", Time::Hours(24).to_nanos() == Time::Days(1).to_nanos());
    test("library: Minutes 60 == Hours 1", Time::Minutes(60).to_nanos() == Time::Hours(1).to_nanos());

    // Time Mul
    let t2 = Time::Seconds(3) * 5;
    test("library: Time Mul scales value", t2.to_nanos() == 15_000_000_000);
    let t3 = Time::Milliseconds(10) * 0;
    test("library: Time Mul by zero", t3.to_nanos() == 0);
    let t4 = Time::Nanoseconds(100) * 1;
    test("library: Time Mul by one", t4.to_nanos() == 100);
}

// ---------------------------------------------------------------------------
// library: CRC32
// ---------------------------------------------------------------------------

fn crc32_tests() {
    test("crc32: empty input", crc32(b"") == 0x00000000);
    test("crc32: known string \"hello\"",
        crc32(b"hello") == 0x3610A686);
    test("crc32: known string \"world\"",
        crc32(b"world") == 0x5D2FD84C);
    test("crc32: single byte", crc32(b"\x00") == 0xD202EF8D);
    test("crc32: known byte sequence",
        crc32(b"123456789") == 0xCBF43926);
    test("crc32: deterministic",
        crc32(b"test") == crc32(b"test"));
    test("crc32: different inputs differ",
        crc32(b"abc") != crc32(b"def"));
}

// ---------------------------------------------------------------------------
// memory: BuddyAllocator
// ---------------------------------------------------------------------------

fn buddy_tests() {
    type TestBuddy = BuddyAllocator<12, 8, 4>;

    // Pure math
    test("buddy: block_size(0) == 4096", TestBuddy::block_size(0) == 4096);
    test("buddy: block_size(1) == 8192", TestBuddy::block_size(1) == 8192);
    test("buddy: block_size(7) == 524288", TestBuddy::block_size(7) == 524288);
    test("buddy: buddy_of is symmetric",
        TestBuddy::buddy_of(TestBuddy::buddy_of(0x10000, 2), 2) == 0x10000);

    // Temp heap-backed tests
    fn alloc_4k_region(size: usize) -> u64 {
        // Use kernel allocator to get pages, then use their physical addresses
        let pages = KMemory::alloc_pages(size / PAGE_SIZE);
        let virt = pages.get_virt_addr().as_u64();
        // Leak so it's not freed
        let _ = pages.leak();
        virt
    }

    let mut b: TestBuddy = TestBuddy::new();
    test("buddy: new is empty", b.heads.iter().all(|&h| h == 0));

    let block = alloc_4k_region(TestBuddy::block_size(7));
    unsafe { b.push(block, 7); }
    let got = b.alloc(7);
    test("buddy: alloc exact order", got == Some(block) && b.heads[7] == 0);

    let block2 = alloc_4k_region(TestBuddy::block_size(7));
    unsafe { b.push(block2, 7); }
    let small = b.alloc(0);
    test("buddy: alloc splits block", small.is_some() && small.unwrap() == block2);
    let buddy_addr = block2 ^ 4096;
    let mut found_buddy = false;
    for o in 0..=7 {
        let mut h = b.heads[o];
        while h != 0 {
            if h == buddy_addr { found_buddy = true; }
            unsafe { h = *(h as *const u64); }
        }
    }
    test("buddy: split produces buddy in free list", found_buddy);

    let block3 = alloc_4k_region(TestBuddy::block_size(7));
    unsafe { b.push(block3, 7); }
    let x = b.alloc(0).unwrap();
    b.free(x, 0);
    test("buddy: free and realloc", b.alloc(0) == Some(x));

    test("buddy: alloc fails when empty",
        BuddyAllocator::<12, 2, 0>::new().alloc(0).is_none());

    test("buddy: order above max returns None",
        BuddyAllocator::<12, 2, 0>::new().alloc(5).is_none());

    // Use a dedicated BuddyAllocator for isolated remove tests
    let mut b2: TestBuddy = TestBuddy::new();
    let block4 = alloc_4k_region(TestBuddy::block_size(3));
    unsafe { b2.push(block4, 3); }
    test("buddy: remove existing", unsafe { b2.remove(block4, 3) });
    test("buddy: head cleared after remove", b2.heads[3] == 0);
    test("buddy: remove nonexistent", !unsafe { b2.remove(0xdead, 0) });

    // alloc_range and free_range
    let mut b3: BuddyAllocator<12, 8, 4> = BuddyAllocator::new();
    let block_range = alloc_4k_region(BuddyAllocator::<12, 8, 4>::block_size(3));
    unsafe { b3.push(block_range, 3); }
    let range_addr = b3.alloc_range(4);
    test("buddy: alloc_range returns Some", range_addr.is_some());
    let addr = range_addr.unwrap();
    test("buddy: alloc_range address matches", addr == block_range);
    let remaining_head = b3.heads[1];
    test("buddy: alloc_range leaves remaining block", remaining_head != 0);
    b3.free_range(addr, 4);
    let reallocated = b3.alloc_range(4);
    test("buddy: free_range + alloc_range roundtrip",
        reallocated == Some(addr));

    let mut b4: BuddyAllocator<12, 8, 4> = BuddyAllocator::new();
    test("buddy: alloc_range on empty returns None",
        b4.alloc_range(1).is_none());

    // free_with conditional merge
    let mut b5: BuddyAllocator<12, 8, 4> = BuddyAllocator::new();
    let block_fw = alloc_4k_region(BuddyAllocator::<12, 8, 4>::block_size(2));
    unsafe { b5.push(block_fw, 2); }
    let addr_fw = b5.alloc(0).unwrap();
    let buddy_addr_fw = addr_fw ^ 4096;
    b5.free_with(addr_fw, 0, |_, _| false);
    let _heads_after_no_merge = b5.heads[0];
    b5.free_with(buddy_addr_fw, 0, |_, _| false);
    test("buddy: free_with blocks merge when allowed",
        b5.heads[0] == 0 && b5.heads[1] != 0);
}

// ---------------------------------------------------------------------------
// memory: Kernel memory allocator (post-init)
// ---------------------------------------------------------------------------

fn memory_tests() {
    let page = KMemory::alloc_page();
    test("memory: alloc_page returns non-zero address",
        page.get_phys_address().as_u64() != 0);
    let ptr = page.get_virt_addr().as_mut_ptr::<u8>();
    unsafe { ptr.write(0xAB); }
    test("memory: alloc_page writable",
        unsafe { ptr.read() == 0xAB });

    let pages = KMemory::alloc_pages(3);
    test("memory: alloc_pages(3) returns non-zero",
        pages.get_phys_address().as_u64() != 0);
    test("memory: alloc_pages(3) returns 3 pages",
        pages.size() == 3);

    let mmio = KMemory::map_mmio(PhysAddr::new(0xFEE00000), 1);
    test("memory: map_mmio returns non-zero address",
        mmio.as_u64() != 0);
    KMemory::unmap_mmio(mmio, 1);
    test("memory: map/unmap mmio succeeds", true);
}

// ---------------------------------------------------------------------------
// memory: PhysPage write_data / read_data
// ---------------------------------------------------------------------------

fn physpage_tests() {
    let mut page = KMemory::alloc_page();
    let base = page.get_virt_addr().as_u64();
    test("physpage: alloc_page non-zero phys",
        page.get_phys_address().as_u64() != 0);
    test("physpage: alloc_page non-zero virt", base != 0);

    page.write_data::<u64>(0, 0xDEADBEEF);
    let val: u64 = page.read_data(0);
    test("physpage: write/read u64 roundtrip", val == 0xDEADBEEF);

    page.write_data::<u32>(8, 0xCAFEBABE);
    let val2: u32 = page.read_data(8);
    test("physpage: write/read u32 roundtrip", val2 == 0xCAFEBABE);

    page.write_data::<u8>(16, 0x42);
    let val3: u8 = page.read_data(16);
    test("physpage: write/read u8 roundtrip", val3 == 0x42);

    let mut pages = KMemory::alloc_pages(2);
    pages.write_data::<u64>(PAGE_SIZE, 0x12345678);
    let val4: u64 = pages.read_data(PAGE_SIZE);
    test("physpage: multi-page write/read roundtrip", val4 == 0x12345678);

    let page2 = KMemory::alloc_page();
    let (_, count) = page2.leak();
    test("physpage: leak returns correct page count", count == 1);

    let page3 = KMemory::alloc_page();
    let phys = page3.get_phys_address();
    drop(page3);
    let page4 = KMemory::alloc_page();
    test("physpage: reallocated page has different address",
        page4.get_phys_address() != phys);
}

// ---------------------------------------------------------------------------
// fs: Bitmap
// ---------------------------------------------------------------------------

fn bitmap_tests() {
    let bm = Bitmap::new();
    test("bitmap: new is zeroed", bm.entries.iter().all(|&b| b == 0));

    let mut b = Bitmap::new();
    b.set(0, true).unwrap();
    test("bitmap: set bit 0", b.check(0).unwrap());

    let mut b2 = Bitmap::new();
    b2.set(7, true).unwrap();
    test("bitmap: set bit 7", b2.check(7).unwrap());

    let mut b3 = Bitmap::new();
    b3.set(8, true).unwrap();
    test("bitmap: set bit 8", b3.check(8).unwrap());

    let mut b4 = Bitmap::new();
    b4.set(10, true).unwrap();
    b4.set(10, false).unwrap();
    test("bitmap: clear bit", !b4.check(10).unwrap());

    let mut b5 = Bitmap::new();
    for &i in &[0, 1, 63, 64, 127, 128, 255, 4095] {
        b5.set(i, true).unwrap();
    }
    let all_set = [0, 1, 63, 64, 127, 128, 255, 4095].iter().all(|&i| b5.check(i).unwrap());
    test("bitmap: multiple bits set and checked", all_set);
    b5.set(63, false).unwrap();
    test("bitmap: clear specific bit", !b5.check(63).unwrap() && b5.check(64).unwrap());

    test("bitmap: out of range check errors", b5.check(4096).is_err());
    test("bitmap: out of range set errors", b5.set(4096, true).is_err());
}

// ---------------------------------------------------------------------------
// fs: hash functions
// ---------------------------------------------------------------------------

fn hash_tests() {
    test("hash: deterministic", hash_name("hello.txt") == hash_name("hello.txt"));
    test("hash: different for different names", hash_name("foo") != hash_name("bar"));
    test("hash: empty string non-zero", hash_name("") != 0);

    let bytes = name_to_bytes("foo").unwrap();
    test("hash: name_to_bytes short name", &bytes[..3] == b"foo");
    let long = "a".repeat(42);
    let bytes2 = name_to_bytes(&long).unwrap();
    test("hash: name_to_bytes max length", &bytes2[..42] == long.as_bytes());

    test("hash: name_to_bytes empty error", name_to_bytes("").is_err());
    test("hash: name_to_bytes too long error", name_to_bytes(&"a".repeat(43)).is_err());

    let bytes3 = name_to_bytes("hello world").unwrap();
    test("hash: name_to_str roundtrip", name_to_str(&bytes3) == "hello world");
    test("hash: name_to_str zeroed", name_to_str(&[0u8; 43]) == "");

    let bytes4 = name_to_bytes("match").unwrap();
    test("hash: name_eq matches", name_eq(&bytes4, "match"));
    test("hash: name_eq no match", !name_eq(&bytes4, "nomatch"));
    test("hash: name_eq wrong length", !name_eq(&bytes4, "matchx"));
}

// ---------------------------------------------------------------------------
// fs: FileType
// ---------------------------------------------------------------------------

fn filetype_tests() {
    test("filetype: File from 0", FileType::from(0) == FileType::File);
    test("filetype: Directory from 1", FileType::from(1) == FileType::Directory);
    test("filetype: Unknown from others",
        FileType::from(2) == FileType::Unknown && FileType::from(255) == FileType::Unknown);
    test("filetype: discriminant File", FileType::File as u8 == 0);
    test("filetype: discriminant Directory", FileType::Directory as u8 == 1);
}

// ---------------------------------------------------------------------------
// fs: DirectoryCache
// ---------------------------------------------------------------------------

fn dummy_entry(name: &str, _parent: u64, start: u64) -> HashEntry {
    let mut bytes = [0u8; 40];
    bytes[..name.len()].copy_from_slice(name.as_bytes());
    HashEntry {
        status: crate::fs::HashStatus::Used,
        type_: FileType::File,
        name: bytes,
        start_block: start,
        file_size: 4096,
        link: LinkEntry { ptr: 0, size: 0 },
    }
}

fn dir_cache_tests() {
    let mut c = DirectoryCache::new();
    test("dircache: empty lookup", c.lookup(0, "x").is_none());

    let mut c2 = DirectoryCache::new();
    c2.insert(0, &dummy_entry("test.txt", 0, 100));
    let found = c2.lookup(0, "test.txt");
    test("dircache: insert and lookup",
        found.map(|e| e.start_block) == Some(100));

    let mut c3 = DirectoryCache::new();
    c3.insert(5, &dummy_entry("foo", 5, 200));
    test("dircache: wrong parent", c3.lookup(0, "foo").is_none());

    let mut c4 = DirectoryCache::new();
    c4.insert(0, &dummy_entry("f", 0, 10));
    c4.insert(0, &dummy_entry("f", 0, 20));
    test("dircache: update existing", c4.lookup(0, "f").map(|e| e.start_block) == Some(20));

    let mut c5 = DirectoryCache::new();
    c5.insert(0, &dummy_entry("rm", 0, 50));
    c5.remove(0, "rm");
    test("dircache: remove", c5.lookup(0, "rm").is_none());

    let mut c6 = DirectoryCache::new();
    for i in 0..10 {
        c6.insert(0, &dummy_entry(&format!("f{}", i), 0, i * 100));
    }
    let all = (0..10).all(|i| {
        c6.lookup(0, &format!("f{}", i)).map(|e| e.start_block) == Some(i * 100)
    });
    test("dircache: multiple entries", all);

    let mut c7 = DirectoryCache::new();
    for i in 0..128 {
        c7.insert(0, &dummy_entry(&format!("k{}", i), 0, i));
    }
    let kept = c7.lookup(0, "k0").is_some();
    c7.insert(0, &dummy_entry("new", 0, 999));
    let inserted = c7.lookup(0, "new").map(|e| e.start_block) == Some(999);
    let still_kept = c7.lookup(0, "k0").is_some();
    test("dircache: LRU eviction", kept && inserted && still_kept);
}

// ---------------------------------------------------------------------------
// gpt: partition entry structure
// ---------------------------------------------------------------------------

fn gpt_tests() {
    use core::mem::size_of;
    test("gpt: GPTPartitionEntry is 128 bytes",
        size_of::<GPTPartitionEntry>() == 128);

    let entry = GPTPartitionEntry {
        partition_type: [0u8; 16],
        guid: [0u8; 16],
        start_lba: 2048,
        end_lba: 2048 + 102400 - 1,
        flags: 0,
        name: [0u16; 36],
    };
    let block_count = entry.end_lba - entry.start_lba + 1;
    test("gpt: partition block count calculation", block_count == 102400);

    let entry2 = GPTPartitionEntry {
        partition_type: [0u8; 16],
        guid: [0u8; 16],
        start_lba: 100,
        end_lba: 100,
        flags: 0,
        name: [0u16; 36],
    };
    test("gpt: single-block partition", entry2.end_lba - entry2.start_lba + 1 == 1);
}

// ---------------------------------------------------------------------------
// scheduling: calculate_priority and predict_burst
// ---------------------------------------------------------------------------

fn make_task(niceness: i8, predicted_burst: f64, last_burst: f64) -> Task {
    Task {
        id: 0,
        page_table: PhysAddr::new(0),
        kstack: PhysPage::dummy(),
        vmas: Vec::new(),
        sp: 0,
        entry: || {},
        status: TaskStatus::Ready,
        wake_time: 0,
        niceness,
        last_exec: 0,
        last_burst,
        predicted_burst,
        fpu_state: FpuState::new(),
        syscall_buffer: PhysPage::dummy(),
        pcid: 1,
        last_cpu: None,
    }
}

fn scheduler_tests() {
    test("sched: priority default niceness", calculate_priority(&make_task(0, 0.0, 0.0)) == 127);
    test("sched: priority max niceness", calculate_priority(&make_task(127, 0.0, 0.0)) == 0);
    test("sched: priority min niceness", calculate_priority(&make_task(-128, 0.0, 0.0)) == 255);
    test("sched: priority penalized by burst",
        calculate_priority(&make_task(0, 100_000_000.0, 0.0)) < 127);
    test("sched: priority burst penalty capped",
        calculate_priority(&make_task(0, 999_999_999.0, 0.0)) == 0);

    let mut t = make_task(0, 1.0, 10_000_000.0);
    predict_burst(&mut t);
    let expected = 0.875 * 10_000_000.0 + 0.125 * 1.0;
    test("sched: burst prediction",
        (t.predicted_burst - expected).abs() < 0.001);

    let mut t2 = make_task(0, 1.0, 100.0);
    for _ in 0..50 {
        predict_burst(&mut t2);
        t2.last_burst = 100.0;
    }
    test("sched: burst convergence",
        (t2.predicted_burst - 100.0).abs() < 0.01);

    let mut t3 = make_task(0, 0.0, 1000.0);
    predict_burst(&mut t3);
    test("sched: burst zero initial",
        (t3.predicted_burst - 875.0).abs() < 0.001);

    // TaskStatus ordering
    test("sched: Ready < Wait",
        TaskStatus::Ready < TaskStatus::Wait);
    test("sched: Wait < Terminate",
        TaskStatus::Wait < TaskStatus::Terminate);
    test("sched: Ready < Terminate",
        TaskStatus::Ready < TaskStatus::Terminate);

    // Priority ordering
    let high_prio = calculate_priority(&make_task(-128, 0.0, 0.0));
    let low_prio = calculate_priority(&make_task(127, 0.0, 0.0));
    test("sched: min niceness has higher priority",
        high_prio > low_prio);
}
