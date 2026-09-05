use limine::{BaseRevision, request::{FramebufferRequest, FramebufferResponse, HhdmRepsonse, HhdmRequest, MemmapRequest, MemmapResponse, MpRequest, MpResponse, RsdpRequest, RsdpResponse}};

use crate::{KernelError, KernelResult, LateInit};

#[used]
#[unsafe(link_section = ".requests")]
pub static REVISION: BaseRevision = BaseRevision::new();

#[used]
#[unsafe(link_section = ".requests")]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static MMAP_REQUEST: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static MP_REQUEST: MpRequest = MpRequest::new(0);

pub static FB_RESPONSE: LateInit<&FramebufferResponse> = LateInit::new();
pub static MMAP_RESPONSE: LateInit<&MemmapResponse> = LateInit::new();
pub static HHDM_RESPONSE: LateInit<&HhdmRepsonse> = LateInit::new();
pub static RSDP_RESPONSE: LateInit<&RsdpResponse> = LateInit::new();
pub static MP_RESPONSE: LateInit<&MpResponse> = LateInit::new();

/// Initialize the response data statics
/// 
/// # Errors
/// Retruns `KernelError::BadLimineResp` on unfulfilled requests
pub fn init() -> KernelResult<()> {
    FB_RESPONSE.init(FRAMEBUFFER_REQUEST.response().ok_or(KernelError::BadLimineResp)?);
    MMAP_RESPONSE.init(MMAP_REQUEST.response().ok_or(KernelError::BadLimineResp)?);
    HHDM_RESPONSE.init(HHDM_REQUEST.response().ok_or(KernelError::BadLimineResp)?);
    RSDP_RESPONSE.init(RSDP_REQUEST.response().ok_or(KernelError::BadLimineResp)?);
    MP_RESPONSE.init(MP_REQUEST.response().ok_or(KernelError::BadLimineResp)?);
    Ok(())
}