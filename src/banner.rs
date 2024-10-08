

/*
 * 7/10/24 Added a struct to abstract the multicolor functions of a Banner S22
 * Centroid ehf. 2024 - Ómar Högni Guðmarsson 
 */



 /*            Red      Yellow      Green       Cyan        Blue        Magenta     White
  * Input 1     X         X                                                X          X
  * Input 2               X           X          X                                    X
  * Input 3                                      X           X             X          X 
  */

pub mod banner {
    use core::fmt;
    use std::path::Display;

pub enum Colors {
    None = 0,
    Red = 1,
    Green  = 2,
    Yellow = 3,
    Blue = 4,
    Magenta = 5,
    Cyan = 6,
    White = 7,
}

pub fn color_to_states(color: Colors) -> (bool, bool, bool) {
    let int_color = color as u32;
    (int_color & 0x1 != 0, int_color & 0x2 != 0, int_color & 0x4 != 0)
}

impl fmt::Display for Colors {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
           Colors::None => write!(f, "None"),
           Colors::Red => write!(f, "Red"),
           Colors::Blue => write!(f, "Blue"),
           Colors::Green => write!(f, "Green"),
           Colors::Cyan => write!(f, "Cyan"),
           Colors::Yellow => write!(f, "Yellow"),
           Colors::Magenta => write!(f, "Magenta"),
           Colors::White => write!(f, "White"),
        }
    }
}
}

#[cfg(test)]
mod tests {
    use super::banner::*;
    #[test]
    fn test_all_states() {
        let (mut i1, mut i2, mut i3) = (false, false, false);
        (i1, i2, i3) = color_to_states(Colors::None);
        assert_eq!(i1, false);
        assert_eq!(i2, false);
        assert_eq!(i3, false);
        (i1, i2, i3) = color_to_states(Colors::Red);
        assert_eq!(i1, true);
        assert_eq!(i2, false);
        assert_eq!(i3, false);
        (i1, i2, i3) = color_to_states(Colors::Green);
        assert_eq!(i1, false);
        assert_eq!(i2, true);
        assert_eq!(i3, false);
        (i1, i2, i3) = color_to_states(Colors::Yellow);
        assert_eq!(i1, true);
        assert_eq!(i2, true);
        assert_eq!(i3, false);
        (i1, i2, i3) = color_to_states(Colors::Blue);
        assert_eq!(i1, false);
        assert_eq!(i2, false);
        assert_eq!(i3, true);
        (i1, i2, i3) = color_to_states(Colors::Magenta);
        assert_eq!(i1, true);
        assert_eq!(i2, false);
        assert_eq!(i3, true);
        (i1, i2, i3) = color_to_states(Colors::Cyan);
        assert_eq!(i1, false);
        assert_eq!(i2, true);
        assert_eq!(i3, true);
        (i1, i2, i3) = color_to_states(Colors::White);
        assert_eq!(i1, true);
        assert_eq!(i2, true);
        assert_eq!(i3, true);

    }
}