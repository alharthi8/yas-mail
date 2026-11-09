# Yas Mail brand

**Yas Mail** — Mail, made clear.

The original **Wing Letter** symbol is a mail-specific sibling to Yas Browser's Falcon Wing. A white envelope, clear blue fold, swept upper wing, and sky-blue rear feather sit on a rounded royal-blue tile. It shares Yas Browser's flat geometric shapes, strong silhouette, and blue palette while remaining distinct as an email application. It uses no letter badge, glow, star, official insignia, or copied falcon silhouette.

## Colors

| Role | Color |
| --- | --- |
| Royal blue: icon tile and envelope fold | `#2563EB` |
| White: envelope and upper wing | `#FFFFFF` |
| Sky blue: rear feather | `#9BC8FF` |
| Navy: wordmark and regular text | `#192C49` |
| Ice: identity presentation surfaces | `#F7F9FC` |
| Slate blue: secondary presentation text | `#5F708A` |

The icon uses a 128-unit square with a 30-unit corner radius. Keep the tile proportions, wing direction, and envelope fold intact. The mark is designed to stay readable at 16 and 24 pixels; do not add fine texture, outlines, or more small details. Use transparent corners, not a solid square surrounding the rounded tile.

## Typography and voice

The wordmark uses **Dubai Bold**, matching the Yas family. SVG wordmarks specify Dubai with Segoe UI as a fallback; the PNG wordmark is rendered with the installed Dubai font. No font files are redistributed. Keep the product name **Yas Mail** with this capitalization.

Use clear, calm, friendly language. Describe actions directly and distinguish provider acceptance from delivery, local demo changes from synchronization, and style examples from model training. Per-account avatar colors remain personal accents without replacing the shared Yas Mail identity.

## Assets

- `yas-mail-symbol.svg`: editable vector master.
- `yas-mail-logo.svg`: identical compatibility alias used by existing documentation.
- `../ui/logo.svg`: identical symbol embedded throughout the application.
- `yas-mail-logo.png`: transparent 1024px master.
- `yas-mail-icon.png`: transparent 256px app mark.
- `yas-mail-symbol-mono.svg`: monochrome mark using `currentColor`.
- `yas-mail-wordmark.svg` and `.png`: Dubai wordmark and Wing Letter lockup.
- `yas-mail.ico` and `../src-tauri/icons/icon.ico`: identical Windows icons with 16, 20, 24, 32, 40, 48, 64, 128, and 256px frames.
- `../src-tauri/icons/32x32.png`, `128x128.png`, and `128x128@2x.png`: platform-ready transparent renditions.
- `yas-mail-brand-preview.png`: identity and small-size review sheet.

Run `python branding/render-assets.py` from the project directory to regenerate PNGs, wordmarks, Windows icons, and the review sheet. It requires PyMuPDF, Pillow, and an installed Dubai Bold font. Use `--font` and `--regular-font` to specify installed font files explicitly; font files are not included in this project.
