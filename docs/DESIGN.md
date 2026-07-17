---
version: alpha
name: Sumi / youtube-sub-feed
description: >
  youtube-sub-feed project overrides for the Sumi design system. The
  canonical template lives at ~/.claude/designs/sumi/DESIGN.md; this file
  records ONLY what is specific to youtube-sub-feed (accent + functional
  data colors + domain components). CSS custom properties in
  client/src/global.sass are the implementation of these tokens.
colors:
  # --- Project accent (YouTube red) ---
  # Unsuffixed = Washi theme (light), -dark = Sumi theme (dark).
  # The Washi value is a dark red ink (white-on-accent ~8:1). It lands
  # close to the template's danger role by necessity — red IS this
  # project's identity — so accent and danger are distinguished by form
  # (fill vs. text), never by hue. See Do's and Don'ts.
  accent: "#9a231b"
  accent-subtle: "rgba(154, 35, 27, 0.12)"
  accent-dark: "#ea4335"
  accent-subtle-dark: "rgba(234, 67, 53, 0.15)"
  # --- Functional data colors (Washi / Sumi pairs) ---
  # Washi values are darkness-ramp inks per the template (ink first, hue
  # as a secondary cue) — provisional seeds pending e-paper tuning.
  # live = broadcasting right now. Pure red, hotter than the accent.
  live: "#7a1414"
  live-subtle: "rgba(122, 20, 20, 0.08)"
  live-dark: "#ff4d4d"
  live-subtle-dark: "rgba(255, 0, 0, 0.15)"
  # shorts = the video is a YouTube Short.
  shorts: "#8f1c5a"
  shorts-subtle: "rgba(143, 28, 90, 0.08)"
  shorts-dark: "#ff6090"
  shorts-subtle-dark: "rgba(255, 96, 144, 0.15)"
  # favorite = a channel saved by the current user.
  favorite: "#8a6000"
  favorite-subtle: "rgba(138, 96, 0, 0.10)"
  favorite-dark: "#d6a632"
  favorite-subtle-dark: "rgba(214, 166, 50, 0.12)"
---

# youtube-sub-feed — Sumi Project Overrides

## Overview

**This project follows the Sumi design system.** The canonical template is
`~/.claude/designs/sumi/DESIGN.md` — all shared rules (neutral chrome,
one-accent rule, typography/spacing/radius scales, flat elevation,
iconography, component recipes) live there and are NOT restated here.
This document records only what is unique to youtube-sub-feed. On chrome
questions the template wins; on the domain semantics below this file wins.

Accent: **YouTube red** (`#9a231b` Washi / `#ea4335` Sumi). Red is the
domain's identity and distinguishes this tool from its amber (5ch-viewer)
and blue (novel-server) siblings. It marks interactive chrome only: the
active group tab underline, primary buttons, focused inputs, the shared
focus ring, the loading spinner.

Themes follow novel-server's Sumi-first convention: `:root` in
`client/src/global.sass` IS the Sumi (dark) theme, and Washi (light,
e-paper) is applied via `@media (prefers-color-scheme: light)` — the OS
decides; there is no in-app toggle and no `data-theme` attribute.

## Colors

Everything below is a **functional data color** in the Sumi sense: it
encodes video state, never decoration, and is exempt from the one-accent
rule. All come in Washi (light) / Sumi (dark) pairs.

This project has three reds with three distinct jobs:

- **Accent (#9a231b / #ea4335):** chrome only — "you are here" and "this
  is the main action". Never appears on a video state badge.
- **Live (#7a1414 / #ff4d4d):** means exactly one thing — "this channel is
  broadcasting right now". Pure, hotter red than the accent. Appears only
  as the LIVE badge on thumbnails (tinted `live-subtle` background, 1px
  live border, bold caption text). In Sumi the badge pulses gently
  (opacity 1 → 0.6, 2s); in Washi it does not animate — e-paper ghosting —
  and bold weight alone carries the urgency.
- **Danger (template role):** destructive text and error states, with no
  extra project meaning.

And one non-red:

- **Shorts (#8f1c5a / #ff6090):** pink/magenta means "this video is a
  Short". Appears only as the Shorts badge on thumbnails, same badge
  anatomy as LIVE. The Washi value is a dark magenta ink.

Ended livestreams (配信アーカイブ) deliberately carry **no data color**:
the state is "over", so the badge renders in neutral chrome (muted text on
a quiet overlay). Only ongoing or format states earn a hue.

## Layout

The template's two-pane list+detail grid does not apply: the "detail" view
of a video is YouTube itself, reached by an external link, so this app has
no detail pane. Instead:

- **Feed:** single column (max 640px, centered) on mobile; at ≥768px it
  becomes a **3-column thumbnail grid**. The thumbnails are the content —
  the grid is the PC-width adaptation of this project.
- **Channels / Settings / Login:** stay single column (max 640px) at every
  width.

## Components

Domain components on top of the Sumi recipes:

- **Video card:** 16:9 thumbnail (cover, md radius) over a two-line
  ellipsized body title and a caption meta row (channel link + relative
  time, muted). The card links out to YouTube in a new tab. Thumbnail
  overlays — the duration pill (bottom-right, white caption on a
  near-black scrim) and the hide button — sit on the image, not the
  chrome, and use scrim-on-image colors per the template's image-viewer
  exception.
- **Status badges (thumbnail top-left):** caption size, bold, tinted
  subtle background + 1px border in the data color. LIVE (live color,
  pulsing in Sumi only), Shorts (shorts color), 配信アーカイブ (neutral
  chrome, see Colors). A video shows at most one badge; Shorts wins over
  livestream states.
- **Hide button (「もう見た」):** quiet scrim overlay button on the
  thumbnail's top-right with an enlarged invisible hit area; hover shifts
  it to the danger role. Hiding is the feed's core gesture — always one
  tap, no confirmation.
- **Play-all bar:** default (non-primary) buttons above the feed that open
  a YouTube queue of the loaded videos — one for normal videos, one for
  Shorts, each showing its count. Play glyphs are SVG per the template.
- **Group nav (Header):** groups are the nav tabs (Sumi tab recipe, active
  = accent underline), horizontally scrollable, with「すべて」first. On
  mobile the tab row is replaced by a native select. Swiping the feed
  left/right cycles through the same group order, quiet per the template's
  gesture rule.
- **Menu (hamburger):** quiet icon button at the header's right opening a
  floating menu (Sumi menu recipe): video list, channel list, group
  management, and the YouTube sync action, which shows busy state by
  disabling itself and swapping its label (同期中...).
- **Channel row:** 40px **circular** avatar — the deliberate domain
  exception to the template's no-circles rule, because a channel icon is
  YouTube identity, not chrome — with title and group names (caption,
  muted), then a trailing action rail: an outlined YT external-link chip
  and delete. A favorite channel has a 2px gold avatar ring plus a small
  star marker (favorite data color, with shape as a non-color cue).
  Right-click or a 500ms touch long-press opens a modal action menu where
  favorite state can be toggled without leaving the list. Deletion is a
  **two-step inline confirm**: a quiet ✕ swaps
  in place to a danger-filled 削除確認 plus a キャンセル escape; no modal.
- **Channel detail:** header with the channel title, a YT external-link
  chip, and **toggle switches** (36×20 pill track, thumb slides right and
  track fills accent when on, label text beside it) for お気に入り and
  ライブ表示, plus ショートNG, which suppresses that channel's Shorts
  from the main feed, RSS, and news feed while leaving them visible in
  channel detail for inspection. Videos here can be hidden AND restored: swipe left hides,
  swipe right restores (quiet swipe-hint panel per the template's gesture
  rule; danger text for hide, accent for restore), hidden videos render at
  50% opacity with a small hidden marker; on PC the same actions appear as
  scrim overlay buttons on thumbnail hover.
- **Group management (Settings):** group rows are Sumi list rows with a
  leading grip handle for drag reordering (drag-over row shows an accent
  border); the name edits in place on click. Group deletion uses the same
  two-step inline confirm as channels. The assignment panel lists channels
  with checkboxes (accent `accent-color`), outlined group-name chips, and
  an accordion row expansion previewing the channel's three latest
  thumbnails.
- **Toast:** fixed bottom-center pill, non-interactive. Success confirms
  briefly and gets out of the way (~0.5s); errors persist longer (~3s) and
  use the danger role (danger-subtle background, danger text/border).
- **Login:** a single centered card explaining the Cloudflare Access
  requirement. No form — authentication happens outside the app.

## Do's and Don'ts

- Do keep the three reds monosemous: accent = chrome, live = broadcasting
  now, danger = destructive/error. Never borrow live red for chrome or
  accent red for a badge.
- Don't rely on hue to tell accent from danger — on Washi they converge
  into near-identical dark red inks by design; form carries the
  distinction (accent fills/underlines, danger stays text and borders).
- Do keep the LIVE pulse in Sumi only; nothing animates in Washi.
- Don't give ended livestreams a data color — archive state is neutral
  chrome.
- Do keep hiding a one-tap action and channel/group deletion a two-step
  confirm: hides are cheap and restorable, deletions are not.
- Do add any new video-state color here (with a Washi/Sumi pair) before
  using it in components.
- Do tune Washi values on an actual e-paper device — the seeds above are
  provisional; contrast over hue.
