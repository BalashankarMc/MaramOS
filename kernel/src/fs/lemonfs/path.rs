use alloc::{string::String, vec::Vec};

pub struct Path(String);

impl Path {
    pub fn is_root(&self) -> bool { self.0 == "/" }
    pub fn components(&self) -> Vec<&str> { self.0.split('/').filter(|c| !c.is_empty() && *c != ".").collect() }
    pub const fn from_string(s: String) -> Self { Self(s) }

    pub fn from_vec(vec: Vec<&str>) -> Self {
        let mut res = String::new();
        res += "/";
        for part in vec { res += part; res += "/" }

        Self(res)
    }

    pub fn parent(&self) -> Self {
        let mut res = self.components();
        res.pop();
        Self::from_vec(res)
    }

    pub fn name(&self) -> String {
        (*self.components().last().unwrap_or(&"/")).into()
    }
}