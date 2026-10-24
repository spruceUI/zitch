Vendored from https://github.com/DGriffin91/egui_software_backend at
c70e02d (0.0.3), for the Miyoo Mini's software-rendered SDL host
(`soft-render`). Licensed MIT OR Apache-2.0, as upstream.

Changed from upstream:
- Cargo.toml: egui 0.34 -> 0.36; the winit and test_render front ends,
  their dependencies, the examples and the workspace removed.
- src/winit.rs, src/test_render.rs and constify/tests removed, and their
  module declarations dropped from src/lib.rs.
- src/lib.rs set_textures: egui 0.36's TexturesDelta.set holds a list of
  ImageDeltas per texture; the loop goes over each.
