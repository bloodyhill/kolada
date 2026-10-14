/// A picture's pixels at the size it is drawn: premultiplied, 0xAARRGGBB,
/// top-down rows.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u32>,
}
