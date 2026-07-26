use crate::requests;
use crate::stdout;

pub fn init() {
    let fb_raw = requests::FRAMEBUFFER_REQUEST.response().unwrap().framebuffers()[0];
    stdout::init(fb_raw);
    stdout::clear();
}
