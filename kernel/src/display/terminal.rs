use core::fmt;
use crate::display::{Color, FrameBuffer};

const TAB_WIDTH: usize = 8;

const FONT: simple_psf::Psf = match simple_psf::Psf::parse(include_bytes!("../../font.psfu")) {
    Ok(f) => f,
    Err(_) => panic!("font.psfu is not a valid PSF file"),
};

pub struct Terminal {
    fb: FrameBuffer,
    cursor_col: usize,
    cursor_row: usize,
    fg: Color,
    bg: Color,
    cols: usize,
    rows: usize,
}

impl Terminal {
    pub const fn new(fb: FrameBuffer) -> Self {
        let cols = fb.width() / FONT.glyph_width;
        let rows = fb.height() / FONT.glyph_height;
        Self {
            fb,
            cursor_col: 0,
            cursor_row: 0,
            fg: Color::WHITE,
            bg: Color::BLACK,
            cols,
            rows,
        }
    }

    pub fn clear(&mut self) {
        self.fb.fill_screen(self.bg.as_u32());
        self.cursor_col = 0;
        self.cursor_row = 0;
    }

    pub fn print_char(&mut self, c: char) {
        match c {
            '\n' => self.advance_row(),
            '\r' => self.cursor_col = 0,
            '\t' => {
                let next = (self.cursor_col / TAB_WIDTH + 1) * TAB_WIDTH;
                if next < self.cols {
                    self.cursor_col = next;
                }
            }
            _ => {
                self.render_glyph(c as usize, self.cursor_col, self.cursor_row);
                self.cursor_col += 1;
                if self.cursor_col >= self.cols {
                    self.cursor_col = 0;
                    self.advance_row();
                }
            }
        }
    }

    pub fn print_str(&mut self, s: &str) {
        for c in s.chars() {
            self.print_char(c);
        }
    }

    pub const fn set_colors(&mut self, fg: Color, bg: Color) {
        self.fg = fg;
        self.bg = bg;
    }

    pub fn set_cursor(&mut self, col: usize, row: usize) {
        self.cursor_col = col.min(self.cols.saturating_sub(1));
        self.cursor_row = row.min(self.rows.saturating_sub(1));
    }

    fn render_glyph(&mut self, glyph_index: usize, col: usize, row: usize) {
        let base_x = col * FONT.glyph_width;
        let base_y = row * FONT.glyph_height;

        if let Some(pixels) = FONT.get_glyph_pixels(glyph_index) {
            for (i, on) in pixels.enumerate() {
                let x = base_x + (i % FONT.glyph_width);
                let y = base_y + (i / FONT.glyph_width);
                let c = if on { self.fg.as_u32() } else { self.bg.as_u32() };
                self.fb.set_pixel(x, y, c);
            }
        } else {
            for gy in 0..FONT.glyph_height {
                for gx in 0..FONT.glyph_width {
                    self.fb.set_pixel(base_x + gx, base_y + gy, self.bg.as_u32());
                }
            }
        }
    }

    fn advance_row(&mut self) {
        self.cursor_row += 1;
        self.cursor_col = 0;
        if self.cursor_row >= self.rows {
            self.scroll();
        }
    }

    fn scroll(&mut self) {
        let row_height = FONT.glyph_height;
        let fb_w = self.fb.width();
        let fb_h = self.fb.height();

        for y in row_height..fb_h {
            for x in 0..fb_w {
                let p = self.fb.read_pixel(x, y);
                self.fb.set_pixel(x, y - row_height, p);
            }
        }

        for y in (fb_h - row_height)..fb_h {
            for x in 0..fb_w {
                self.fb.set_pixel(x, y, self.bg.as_u32());
            }
        }

        self.cursor_row = self.rows - 1;
    }
}

impl fmt::Write for Terminal {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.print_str(s);
        Ok(())
    }
}
