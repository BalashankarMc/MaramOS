use limine::{request::FramebufferRequest, BaseRevision};

#[used]
pub static REVISION: BaseRevision = BaseRevision::new();

#[used]
pub static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new(); 