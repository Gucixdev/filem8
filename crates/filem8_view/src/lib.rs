//! filem8_view: look at any file, never change it. Zoom, rotate, flip, pan, loop animations, walk the folder.
//! Everything the viewer shows becomes pages of pixels (an image is its frames; text, archive listings, hex dumps and
//! waveforms are drawn onto a page; a 3D model is rendered), so one zoom/rotate/loop engine serves every format.
//!   xform   the zoom / rotate / flip / pan maths (pure)
//!   anim    frame timing and looping (pure)
//!   doc     file -> pages, using the same loaders as filem8's preview pane
//!   render  pages -> pixels
//!   nav     the files next to the one you opened
pub mod anim;
pub mod doc;
pub mod nav;
pub mod render;
pub mod xform;
