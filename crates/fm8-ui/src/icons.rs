//! Procedural icons: drawn from rounded rects and lines so they scale with
//! DPI and follow the theme colours, no icon-theme dependency.

use crate::draw::DrawList;
use crate::geom::{rect, Color, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Folder, File, Image, Audio, Video, Archive, Code, Cube, Text, Pdf, Disk, Home, Star, Trash, Download,
    Desktop, Document, Music, Picture, Clock,
    Back, Forward, Up, Plus, Close, Search, Refresh, Grid, List, Compact, Split, Eye, Play, Pause, Menu,
}

/// Draw `icon` fitted in the square `r` using `c` (and `c2` as accent).
pub fn draw(dl: &mut DrawList, icon: Icon, r: Rect, c: Color, c2: Color) {
    let s = r.w.min(r.h);
    let r = r.centered(s, s);
    let u = s / 16.0; // 16-unit design grid
    let at = |x: f32, y: f32| (r.x + x * u, r.y + y * u);
    let w = (1.5 * u).max(1.0);
    match icon {
        Icon::Folder | Icon::Home | Icon::Desktop | Icon::Document | Icon::Music | Icon::Picture | Icon::Download => {
            dl.rounded(rect(r.x + 1.0 * u, r.y + 2.5 * u, 6.0 * u, 3.5 * u), c2, 1.2 * u);
            dl.rounded(rect(r.x + 1.0 * u, r.y + 4.0 * u, 14.0 * u, 10.0 * u), c, 1.8 * u);
            let k = Color { a: 255, ..c2 };
            match icon {
                Icon::Home => dl.rounded(rect(r.x + 6.5 * u, r.y + 8.0 * u, 3.0 * u, 4.0 * u), k, 0.6 * u),
                Icon::Download => {
                    dl.line(at(8.0, 6.5), at(8.0, 11.5), w, k);
                    dl.line(at(5.5, 9.2), at(8.0, 11.7), w, k);
                    dl.line(at(10.5, 9.2), at(8.0, 11.7), w, k);
                }
                Icon::Music => dl.rounded(rect(r.x + 7.0 * u, r.y + 7.0 * u, 3.0 * u, 3.0 * u), k, 1.5 * u),
                Icon::Picture => dl.rounded(rect(r.x + 5.0 * u, r.y + 7.0 * u, 6.0 * u, 4.0 * u), k, 0.8 * u),
                Icon::Document => {
                    dl.line(at(5.0, 8.0), at(11.0, 8.0), w, k);
                    dl.line(at(5.0, 10.5), at(11.0, 10.5), w, k);
                }
                Icon::Desktop => dl.stroke(rect(r.x + 4.5 * u, r.y + 6.5 * u, 7.0 * u, 5.0 * u), k, 0.8 * u, w),
                _ => {}
            }
        }
        Icon::File | Icon::Text | Icon::Code | Icon::Pdf | Icon::Image | Icon::Audio | Icon::Video | Icon::Archive | Icon::Cube => {
            dl.rounded(rect(r.x + 3.0 * u, r.y + 1.0 * u, 10.0 * u, 14.0 * u), c, 1.5 * u);
            dl.rounded(rect(r.x + 9.0 * u, r.y + 1.0 * u, 4.0 * u, 4.0 * u), c2, 1.0 * u);
            match icon {
                Icon::Text | Icon::File => {
                    for i in 0..3 {
                        dl.line(at(5.0, 7.5 + 2.2 * i as f32), at(11.0, 7.5 + 2.2 * i as f32), w * 0.7, c2);
                    }
                }
                Icon::Code => {
                    dl.line(at(7.0, 8.0), at(5.0, 10.0), w * 0.8, c2);
                    dl.line(at(5.0, 10.0), at(7.0, 12.0), w * 0.8, c2);
                    dl.line(at(9.0, 8.0), at(11.0, 10.0), w * 0.8, c2);
                    dl.line(at(11.0, 10.0), at(9.0, 12.0), w * 0.8, c2);
                }
                Icon::Image => {
                    dl.rounded(rect(r.x + 5.0 * u, r.y + 9.0 * u, 6.0 * u, 4.0 * u), c2, 0.8 * u);
                    dl.rounded(rect(r.x + 5.5 * u, r.y + 6.0 * u, 2.0 * u, 2.0 * u), c2, 1.0 * u);
                }
                Icon::Audio => {
                    dl.line(at(6.0, 8.0), at(6.0, 12.5), w * 0.8, c2);
                    dl.line(at(8.0, 6.5), at(8.0, 13.5), w * 0.8, c2);
                    dl.line(at(10.0, 9.0), at(10.0, 11.5), w * 0.8, c2);
                }
                Icon::Video => {
                    dl.line(at(6.5, 7.5), at(6.5, 12.5), w, c2);
                    dl.line(at(6.5, 7.5), at(11.0, 10.0), w * 0.8, c2);
                    dl.line(at(6.5, 12.5), at(11.0, 10.0), w * 0.8, c2);
                }
                Icon::Archive => {
                    dl.rect(rect(r.x + 7.2 * u, r.y + 6.0 * u, 1.6 * u, 8.0 * u), c2);
                    dl.rect(rect(r.x + 6.0 * u, r.y + 9.0 * u, 4.0 * u, 1.4 * u), c2);
                }
                Icon::Cube => {
                    dl.stroke(rect(r.x + 5.5 * u, r.y + 8.0 * u, 5.0 * u, 5.0 * u), c2, 0.6 * u, w * 0.7);
                    dl.line(at(5.5, 8.0), at(7.0, 6.8), w * 0.7, c2);
                    dl.line(at(10.5, 8.0), at(12.0, 6.8), w * 0.7, c2);
                }
                Icon::Pdf => dl.rounded(rect(r.x + 5.0 * u, r.y + 9.5 * u, 6.0 * u, 3.0 * u), c2, 0.8 * u),
                _ => {}
            }
        }
        Icon::Disk => {
            dl.rounded(rect(r.x + 1.0 * u, r.y + 5.0 * u, 14.0 * u, 7.0 * u), c, 2.0 * u);
            dl.rounded(rect(r.x + 11.0 * u, r.y + 8.0 * u, 2.0 * u, 2.0 * u), c2, 1.0 * u);
        }
        Icon::Star => {
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = std::f32::consts::PI * (i as f32 / 5.0) - std::f32::consts::FRAC_PI_2;
                    let rr = if i % 2 == 0 { 7.0 } else { 3.0 };
                    at(8.0 + rr * a.cos(), 8.2 + rr * a.sin())
                })
                .collect();
            for i in 0..10 {
                dl.line(pts[i], pts[(i + 1) % 10], w * 0.8, c);
            }
        }
        Icon::Trash => {
            dl.rounded(rect(r.x + 3.5 * u, r.y + 5.0 * u, 9.0 * u, 9.0 * u), c, 1.2 * u);
            dl.rounded(rect(r.x + 2.5 * u, r.y + 3.0 * u, 11.0 * u, 1.6 * u), c, 0.6 * u);
            dl.rect(rect(r.x + 6.0 * u, r.y + 1.5 * u, 4.0 * u, 1.5 * u), c);
        }
        Icon::Clock => {
            dl.stroke(rect(r.x + 1.5 * u, r.y + 1.5 * u, 13.0 * u, 13.0 * u), c, 6.5 * u, w);
            dl.line(at(8.0, 8.0), at(8.0, 4.5), w, c);
            dl.line(at(8.0, 8.0), at(10.5, 9.5), w, c);
        }
        Icon::Back => {
            dl.line(at(13.0, 8.0), at(3.5, 8.0), w, c);
            dl.line(at(3.5, 8.0), at(8.0, 3.5), w, c);
            dl.line(at(3.5, 8.0), at(8.0, 12.5), w, c);
        }
        Icon::Forward => {
            dl.line(at(3.0, 8.0), at(12.5, 8.0), w, c);
            dl.line(at(12.5, 8.0), at(8.0, 3.5), w, c);
            dl.line(at(12.5, 8.0), at(8.0, 12.5), w, c);
        }
        Icon::Up => {
            dl.line(at(8.0, 13.0), at(8.0, 3.5), w, c);
            dl.line(at(8.0, 3.5), at(3.5, 8.0), w, c);
            dl.line(at(8.0, 3.5), at(12.5, 8.0), w, c);
        }
        Icon::Plus => {
            dl.line(at(8.0, 3.0), at(8.0, 13.0), w, c);
            dl.line(at(3.0, 8.0), at(13.0, 8.0), w, c);
        }
        Icon::Close => {
            dl.line(at(4.0, 4.0), at(12.0, 12.0), w, c);
            dl.line(at(12.0, 4.0), at(4.0, 12.0), w, c);
        }
        Icon::Search => {
            dl.stroke(rect(r.x + 2.0 * u, r.y + 2.0 * u, 9.0 * u, 9.0 * u), c, 4.5 * u, w);
            dl.line(at(9.5, 9.5), at(14.0, 14.0), w, c);
        }
        Icon::Refresh => {
            dl.stroke(rect(r.x + 2.5 * u, r.y + 2.5 * u, 11.0 * u, 11.0 * u), c, 5.5 * u, w);
            dl.rect(rect(r.x + 7.0 * u, r.y + 1.0 * u, 7.0 * u, 4.0 * u), Color::TRANSPARENT);
            dl.line(at(9.0, 2.5), at(12.5, 2.5), w, c);
            dl.line(at(12.5, 2.5), at(12.5, 6.0), w, c);
        }
        Icon::Grid => {
            for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                dl.rounded(rect(r.x + (2.5 + 6.0 * i as f32) * u, r.y + (2.5 + 6.0 * j as f32) * u, 5.0 * u, 5.0 * u), c, 1.0 * u);
            }
        }
        Icon::List => {
            for i in 0..3 {
                let y = 3.5 + 4.0 * i as f32;
                dl.rounded(rect(r.x + 2.0 * u, r.y + y * u, 2.0 * u, 2.0 * u), c, 0.6 * u);
                dl.line(at(6.0, y + 1.0), at(14.0, y + 1.0), w, c);
            }
        }
        Icon::Compact => {
            for i in 0..4 {
                let y = 2.5 + 3.2 * i as f32;
                dl.line(at(2.0, y + 0.6), at(7.0, y + 0.6), w * 0.8, c);
                dl.line(at(9.0, y + 0.6), at(14.0, y + 0.6), w * 0.8, c);
            }
        }
        Icon::Split => {
            dl.stroke(rect(r.x + 1.5 * u, r.y + 2.5 * u, 13.0 * u, 11.0 * u), c, 1.5 * u, w);
            dl.line(at(8.0, 2.5), at(8.0, 13.5), w, c);
        }
        Icon::Eye => {
            dl.stroke(rect(r.x + 1.0 * u, r.y + 4.5 * u, 14.0 * u, 7.0 * u), c, 3.5 * u, w);
            dl.rounded(rect(r.x + 6.0 * u, r.y + 6.0 * u, 4.0 * u, 4.0 * u), c, 2.0 * u);
        }
        Icon::Play => {
            dl.line(at(5.0, 3.5), at(5.0, 12.5), w, c);
            dl.line(at(5.0, 3.5), at(12.5, 8.0), w, c);
            dl.line(at(5.0, 12.5), at(12.5, 8.0), w, c);
        }
        Icon::Pause => {
            dl.rounded(rect(r.x + 4.0 * u, r.y + 3.0 * u, 3.0 * u, 10.0 * u), c, 0.8 * u);
            dl.rounded(rect(r.x + 9.0 * u, r.y + 3.0 * u, 3.0 * u, 10.0 * u), c, 0.8 * u);
        }
        Icon::Menu => {
            for i in 0..3 {
                dl.line(at(2.5, 4.0 + 4.0 * i as f32), at(13.5, 4.0 + 4.0 * i as f32), w, c);
            }
        }
    }
}
