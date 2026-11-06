//! PNG of a part of a rendered window: the RGBA buffer Slint's
//! `Window::take_snapshot` returns, cropped to the view's rectangle (in
//! physical pixels: logical position × `window.scale_factor()`).

use std::fmt;

use slint::{Rgba8Pixel, SharedPixelBuffer};

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// The rectangle has no pixels.
    Empty,
    /// The rectangle reaches outside the `width` × `height` buffer.
    OutOfBounds {
        rect: Rect,
        width: u32,
        height: u32,
    },
    Encode(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::Empty => write!(f, "the image area is empty"),
            ImageError::OutOfBounds {
                rect,
                width,
                height,
            } => write!(
                f,
                "the image area {}×{} at {},{} lies outside the {width}×{height} window image",
                rect.width, rect.height, rect.x, rect.y
            ),
            ImageError::Encode(e) => write!(f, "PNG encoding failed: {e}"),
        }
    }
}

impl std::error::Error for ImageError {}

/// `rect` of `buffer` as PNG bytes (8-bit RGBA).
pub fn png(buffer: &SharedPixelBuffer<Rgba8Pixel>, rect: Rect) -> Result<Vec<u8>, ImageError> {
    let (width, height) = (buffer.width(), buffer.height());
    if rect.width == 0 || rect.height == 0 {
        return Err(ImageError::Empty);
    }
    let fits =
        |start: u32, len: u32, max: u32| start.checked_add(len).is_some_and(|end| end <= max);
    if !fits(rect.x, rect.width, width) || !fits(rect.y, rect.height, height) {
        return Err(ImageError::OutOfBounds {
            rect,
            width,
            height,
        });
    }
    let bytes = buffer.as_bytes();
    let stride = width as usize * 4;
    let mut pixels = Vec::with_capacity(rect.width as usize * rect.height as usize * 4);
    for y in rect.y..rect.y + rect.height {
        let start = y as usize * stride + rect.x as usize * 4;
        pixels.extend_from_slice(&bytes[start..start + rect.width as usize * 4]);
    }
    encode(&pixels, rect.width, rect.height).map_err(|e| ImageError::Encode(e.to_string()))
}

fn encode(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(pixels)?;
    writer.finish()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4×3 image whose pixel (x, y) is (x, y, 7, 255).
    fn buffer() -> SharedPixelBuffer<Rgba8Pixel> {
        let mut b = SharedPixelBuffer::<Rgba8Pixel>::new(4, 3);
        for (i, p) in b.make_mut_slice().iter_mut().enumerate() {
            *p = Rgba8Pixel {
                r: (i % 4) as u8,
                g: (i / 4) as u8,
                b: 7,
                a: 255,
            };
        }
        b
    }

    #[test]
    fn png_round_trip_has_the_cropped_size_and_pixels() {
        let rect = Rect {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };
        let bytes = png(&buffer(), rect).expect("png");
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().expect("header");
        let mut data = vec![0; reader.output_buffer_size().expect("size")];
        let info = reader.next_frame(&mut data).expect("frame");
        assert_eq!((info.width, info.height), (2, 2));
        assert_eq!(
            &data[..info.buffer_size()],
            &[1, 1, 7, 255, 2, 1, 7, 255, 1, 2, 7, 255, 2, 2, 7, 255]
        );
    }

    #[test]
    fn png_rejects_empty_and_outside_areas() {
        assert_eq!(
            png(
                &buffer(),
                Rect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 1
                }
            ),
            Err(ImageError::Empty)
        );
        let outside = Rect {
            x: 3,
            y: 0,
            width: 2,
            height: 1,
        };
        let err = png(&buffer(), outside).unwrap_err();
        assert_eq!(
            err,
            ImageError::OutOfBounds {
                rect: outside,
                width: 4,
                height: 3
            }
        );
        assert!(err.to_string().contains("outside the 4×3"), "{err}");
    }
}
