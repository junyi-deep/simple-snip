---
name: Simple Snip
description: Quiet native screenshot tools that keep the image dominant.
colors:
  primary: "#171717"
  primary-foreground: "#fafafa"
  background: "#ffffff"
  foreground: "#0a0a0a"
  muted: "#f5f5f5"
  border: "#e5e5e5"
  selection-cyan: "#28d4de"
  overlay-dim: "#06101b88"
  canvas-panel: "#15212d"
  canvas-panel-foreground: "#f5fafc"
  helper: "#596977"
  row-border: "#dde3e8"
  annotation-red: "#f03c46"
  annotation-yellow: "#ffb41e"
  annotation-green: "#1eb46e"
  annotation-blue: "#1e82f0"
  annotation-white: "#ffffff"
  annotation-black: "#141414"
typography:
  title:
    fontFamily: ".SystemUIFont"
    fontSize: "1.5rem"
    fontWeight: 600
  body:
    fontFamily: ".SystemUIFont"
    fontSize: "0.875rem"
  label:
    fontFamily: ".SystemUIFont"
    fontSize: "0.75rem"
rounded:
  toolbar: "6px"
spacing:
  icon-gap: "1px"
  compact: "4px"
  image-toolbar-gap: "6px"
  unit: "8px"
  row: "12px"
  content: "24px"
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.primary-foreground}"
  button-tool:
    size: "20px"
  annotation-toolbar:
    rounded: "{rounded.toolbar}"
    padding: "{spacing.compact}"
  settings-nav:
    backgroundColor: "{colors.muted}"
    width: "200px"
---

# Design System: Simple Snip

## Overview

**Creative North Star: "The Quiet Native Workbench"**

Simple Snip is a compact native desktop utility. Its visual character is practical, restrained, and precise: system typography, quiet preferences, familiar GPUI Kit controls, and image-first capture and pin surfaces. Controls support the image without turning annotation into a separate application frame.

This document merges the existing design guidance with the confirmed 2026-10-25 correction and the current Rust implementation. It describes GPUI Kit (0.8.0) on Windows AMD64 and macOS ARM64. The North Star is descriptive shorthand for the incumbent system; it is not a new user-selected brand identity. After unlocking, macOS native checks verified inline rectangle/arrow annotation, undo/redo, toolbar below/inside placement, copy-to-pin, and finish/cancel focus recovery. Remaining tools, dark theme, small images and Windows still need acceptance; measurements remain source-observed unless explicitly verified.

**Key Characteristics:**

- Native system typography and semantic Kit theme colors.
- Compact independent geometric icons with names and tooltips.
- Annotation in the screenshot overlay or existing selected pin.
- A small toolbar tight against the image's lower-right edge.
- Quiet preferences with descriptive rows and actionable recovery text.

## Colors

Neutral Kit surfaces provide the working environment; cyan identifies the capture boundary, while annotation colors belong to image content.

### Primary

The frontmatter's `primary` and `primary-foreground` are the installed Kit Default Light values, extracted from the pinned dependency. They document that theme's reference appearance. Production controls must continue to use the active Kit theme roles, so light/dark changes update them. Do not hardcode these light values into Rust controls.

### Neutral

`background`, `foreground`, `muted`, and `border` likewise record Kit Default Light reference values. Preferences use background/foreground; the sidebar uses muted; dividers use border. The annotation toolbar uses the active theme's popover/popover_foreground roles, whose light appearance matches background/foreground.

The canvas uses a fixed dark slate panel (`canvas-panel`) and pale panel text (`canvas-panel-foreground`) for capture instructions and the magnifier. `overlay-dim` subdues the desktop outside the selected region. Preferences currently use fixed helper and row-border colors; these are implementation exceptions, not a replacement for semantic Kit roles, and their dark-theme legibility still needs native review.

### Canvas and annotation colors

`selection-cyan` traces the selected screenshot rectangle. The six annotation colors are content ink; red is initially active. OCR uses translucent image overlays: selected words (RGBA `0x2597f785`), hovered words (`0x2597f74c`), and recognized words (`0x49d9dc18`). These fixed canvas overlays must remain distinguishable from the screenshot itself.

**The Semantic Surface Rule.** Resolve application surfaces and control states through Kit's active semantic theme; fixed image overlays and annotation ink retain their content-specific colors.

## Typography

The application inherits the platform system family through Kit's `.SystemUIFont` resolution. Use the platform-resolved family and CJK fallback for Chinese labels; there is no separate display or brand typeface.

- **Title:** Semibold section headings use the `title` role. The current preferences implementation uses GPUI `text_2xl` (1.5rem); this supersedes the earlier document's 22px estimate.
- **Body:** Preferences use the `body` role (0.875rem, nominally 14px with a 16px rem basis).
- **Label:** Helper text, toolbar messages, and pixel instructions use `label` (0.75rem, nominally 12px).
- **Application name:** The sidebar name uses Kit's large text tier (1.125rem) and semibold weight.

Keep tool names in accessibility labels and tooltips, even when the visible control is only an icon. Avoid decorative uppercase tracking and editorial display type. Annotation text is image content, sized separately from interface labels.

## Layout

Preferences open at (900 × 660 logical pixels), with a minimum size of (760 × 540). A 200px scrollable sidebar sits beside a scrolling content column. Preserve the established spacing rhythm: an 8px unit, 24px content inset, 12px row padding, and 4px local gaps. Rows place a title and helper description on the left and a control on the right. The status/save strip remains below the content.

The screenshot overlay occupies one display. The selected image stays in its original selection, with annotation embedded there. Pins are independent image windows; initial fit is bounded by (680 × 500) without enlarging smaller images. Preserve image aspect and source-pixel coordinate mapping when scaled or transformed.

**The Lower-Right Attachment Rule.** Align the compact toolbar with the image's lower-right edge, separated by the `image-toolbar-gap`. Place it below when the display has room; otherwise place it inside the image, above its lower border. Clamp to display bounds instead of clipping controls at the image edge.

The toolbar's preferred width is (520px). Capture placement uses available viewport width, a lower sizing bound (260px), and horizontal margins (8px). Pin toolbar placement caps width to display width minus (16px), with horizontal margins (8px) and vertical margins (6px). These are native available-space rules; the application has no web/mobile breakpoints.

The main icon strip wraps. Contextual options and status messages increase toolbar height. Current sizing reserves a base (34px), an option row (38px), and a message allowance (26px in capture placement; 32px for the pin popup). Wrapped strip height on unusually narrow displays remains a visual verification item.

## Elevation & Depth

Preferences rely on tonal layering and thin separators. The annotation toolbar uses the active popover surface and GPUI's medium shadow; the shadow separates controls from arbitrary screenshot content. Capture dimming, the cyan outline, and dark utility panels supply depth on the canvas. Native window stacking keeps pins and their toolbar visible without an additional image-editor shell.

Exact native shadow rendering and compositor behavior have not been visually measured. Keep the existing Kit/GPUI shadow token instead of substituting an invented CSS shadow in production. No app-specific animation system has been established.

## Shapes

Pins show no outline while inactive. Selecting a pin reveals its configurable blue outline (default #1688FF, 1 logical pixel); focusing that pin's annotation toolbar retains the outline. The selection frame is interface state and is never baked into exported pixels. This supersedes the always-visible border in the v0.0.2 release.

Use modest rounded native controls, rectangular image bounds, and precise geometric annotation icons. The annotation toolbar uses GPUI's medium rounded style (6px at the default rem basis); Kit buttons retain theme-controlled corner radii. The image itself receives no decorative card frame.

Icons are original inline SVG geometry on a (24 × 24) viewBox, nominally (20 × 20), with a (1.65) stroke, rounded caps, and rounded joins. Keep one consistent stroke language across selection, shapes, text, OCR, save, copy, pin, and dismissal. Color choices are small circular swatches with a subtle stroke.

## Components

### Buttons

Use Kit buttons for all actionable controls. Primary buttons identify the current preferences section and the main action, such as capture or save settings. Secondary actions retain standard Kit styling. Toolbar controls use extra-small ghost icon buttons (20px square in the pinned Kit implementation); selection uses Kit's selected treatment. Hover, focus, and disabled behavior belong to the Kit component, with accessible names and tooltips preserved.

### Inputs / Fields

Use Kit Input/InputState for shortcuts, numbers, paths, OCR corrections, and annotation text. Preferences use constrained control widths appropriate to the value: narrow numeric fields, wider key bindings, and a wider save-directory field. The toolbar uses extra-small inputs. Keep parsing and permission errors in the existing status/message surface with a useful recovery action or instruction.

### Navigation

Preferences use a vertical stack of named Kit buttons, with the active section primary. The sidebar remains quiet and dense; the selected title is repeated as a clear content heading. The footer status does not compete with the page heading.

### Annotation toolbar

A rounded popover contains an icon strip, a thin divider between tools and actions, and only the currently relevant options. The tool set covers selection, rectangle, ellipse, line, arrow, pen, highlight, text, numbered markers, mosaic, blur, and eraser. Actions include undo/redo, local OCR, save, copy, pin, and cancellation; completion is available when editing a pin.

Show color and width choices when annotation is active; mount the text field at the clicked image position for the text tool; Enter commits and Escape cancels the input. Arrow options expose three body-width styles. Selected objects accept wheel thickness changes. OCR replaces annotation options with selection/all/TSV copy and line correction. Keep status text within the toolbar rather than creating a separate editor window.

**The One Image Surface Rule.** Edit in the current screenshot selection or original selected pin. A pin may open a toolbar-only native popup so even a tiny pin retains the complete controls; that popup must never duplicate the image canvas or become an editor application frame.

### Capture selection and magnifier

Dim everything outside the selection, keep the image clear inside, and draw a fine cyan boundary (1px). Before annotation, a nearest-neighbor pixel magnifier displays a source crop (up to 13 × 13 pixels) at (104 × 104), with a compact HEX-copy hint. Capture instructions are small dark panels bounded to the display.

### Pin feedback

A pin displays image content without a persistent toolbar until editing. Copy feedback appears as a small dark translucent notice at its lower-right edge and clears after (4 seconds). Finishing or cancelling annotation hides and clears the reusable toolbar-only popup and leaves the original pin as the image surface.

## Do's and Don'ts

### Do:

- **Do** keep screenshots and pins visually dominant.
- **Do** attach tools to the image's lower-right edge and move them inside above the lower border when space below is unavailable.
- **Do** preserve the toolbar-only native float for pins so tiny images do not crop its controls.
- **Do** use semantic Kit surface/control colors and the platform system font.
- **Do** keep tooltips, accessible names, keyboard access, and actionable recovery messages.
- **Do** verify final toolbar placement, wrapping, focus, and dark-theme contrast in unlocked native sessions on each supported platform.

### Don't:

- **Don't** open a separate image editor window or duplicate a pin's image into a second editor frame.
- **Don't** place a large application chrome, title bar, or settings sidebar around annotation.
- **Don't** let the image's clipping bounds crop the toolbar.
- **Don't** copy proprietary Snipaste icons or visual assets; retain the original crop-corner app icon and independent tool geometry.
- **Don't** treat the sidecar's HTML previews as proof of native rendering or completed visual acceptance.

## 0.1.1 feedback refinement

The app mark uses a blue `#1769e0` tile and white crop brackets with a tighter gap. App and tray assets derive from the same original geometry. Image text uses an in-place field, never a toolbar text-entry row (OCR correction remains contextual). Numbered markers and arrow body-style controls share the existing compact toolbar. Tool/range hints follow the image pointer and are excluded from exported pixels. Desktop trail Root and native window backgrounds must both be transparent; paths use cubic curves without repeated sample-point cap overlays; default fade lifetime is 300ms. Native permission recovery uses a system message box with a direct settings action.

## Public 0.0.1 settings

About includes an explicit GitHub update check, download progress, verified package reveal, and system/direct/custom HTTP proxy modes. Address and optional username/password fields appear only for custom proxy. Password is masked and session-only. Trail lifetime remains numeric 100–3000ms and reapplies immediately on save; the continuous ribbon interpolates color, alpha, and width across shared edges.

## 0.0.2 native refinement

Pins have a default thin blue #1688FF frame. Color, 0–8px width and shadow can be changed in Pin settings. Transparent padding accommodates the shadow without changing image coordinates or exported pixels. Toolbar controls align to the actual image right edge, excluding shadow padding; natural icon widths replace the reserved empty area. Eight blue/white handles remain visible on a selected capture and preserve annotation coordinates when resized. Preferences explicitly show and rise above own pins when activated, then return to normal level on blur. Final unlocked visual acceptance is pending.

## Live capture controls (v0.0.3)

Long capture and GIF use a compact borderless control strip attached to the selected region’s lower-right edge, with the same below/inside placement rule as annotation. Status and controls wrap; long messages scroll within the strip. A live capture keeps its controls usable. Settings descriptions wrap inside a flexible min-width-zero text column; fixed-size controls retain their width. Both navigation and main content can scroll at the minimum window size.

The original crop mark now uses circular corner arcs and round caps. The app surface follows the supplied reference #2563EB blue, 48px outer inset and 208px outer radius on a 1024px canvas. macOS uses an independent black-alpha menu-bar template at 18 logical pixels; Windows retains a colored app icon for taskbar contrast.
