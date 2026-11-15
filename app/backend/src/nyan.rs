use crate::github::bytes;

const CAT: &str = "https://www.nyan.cat/cats/original.gif";

pub fn cat() -> Option<Vec<u8>> {
    bytes(CAT).ok()
}
