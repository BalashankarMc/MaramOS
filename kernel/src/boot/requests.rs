//! Limine boot protocol request statics.
//!
//! Declares and initializes the Limine requests (base revision, framebuffer,
//! memory map, HHDM, RSDP, MP) and wraps their responses in lazy statics
//! for early-boot access.

use lazy_static::lazy_static;
use limine::BaseRevision;
use limine::mp::MpRespData;
use limine::request::{
    FramebufferRequest, FramebufferRespData, HhdmRequest, MemmapRequest, MemmapRespData, MpRequest,
    Response, RsdpRequest, RsdpRespData,
};

#[used]
#[unsafe(link_section = ".requests")]
pub static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[unsafe(link_section = ".requests")]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static MEMORY_MAP: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static HHDM: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static RSDP: RsdpRequest = RsdpRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static MP: MpRequest = MpRequest::new(0);

lazy_static! {
    pub static ref FB_DATA: &'static Response<FramebufferRespData> = FRAMEBUFFER_REQUEST.response().expect("Failed to get FB Data");
    pub static ref MMAP: &'static Response<MemmapRespData> = MEMORY_MAP.response().expect("Failed to get Memory Map data");
    pub static ref HHDM_OFFSET: u64 = HHDM.response().expect("Failed to get HHDM Offset!").offset;
    pub static ref RSDP_DATA: &'static Response<RsdpRespData> = RSDP.response().expect("Failed to get RSDP Data");
    pub static ref MP_DATA: &'static Response<MpRespData> = MP.response().expect("Failed to get MP Data");
}
