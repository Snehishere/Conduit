# Conduit Desktop — Design Spec

> Originally written 2026-09-13. **Revised against the implementation on
> 2026-09.** This file is the desktop UI's design record: it keeps design intent
> that has not been built, and it marks intent that has drifted from the code.
> It does not duplicate `docs/ARCHITECTURE.md`, which owns the system, or
> the project Limitations section, which lists what is not built.

## How to read this document

Every design section carries a status. The statuses are not decoration — the
2026-09 audit found that documentation in this repository asserted things that
were checkable-looking and untrue, so a design spec that cannot be checked
against the source is worse than no spec at all.

| Status | Meaning |
|---|---|
| **Shipped** | Implemented. The description below is what the code does, verified against the file and line cited. |
| **Shipped, drifted** | Implemented, but the implementation differs from what this document originally specified. Both are stated, with the reason where the code explains one. |
| **Not implemented** | Design intent recorded here, not built. Tracked in the project Limitations section. |
| **Dropped** | Intent that the code deliberately moved away from. Kept so the reversal is not rediscovered and re-litigated. |

Where a section is **Not implemented**, the design intent is preserved verbatim
in substance. A design spec is allowed to describe a target. It is not allowed
to describe a target as though it were shipped.

---

## Design Philosophy

Conduit's desktop UI should feel **alive** — not a static dashboard, but a living
ecosystem that mirrors the real relationship between your devices. Three core
ideas:

1. **Organic motion** — Things float, drift, and settle. Nothing snaps to a grid
   rigidly.
2. **Gesture-first** — Swipe before you click. Actions should feel physical.
3. **Content-adaptive** — Let the data decide its own shape. Don't force
   everything into rows.

> **Superseded in part (see §1).** Idea 1 as originally written — ambient,
   self-sustaining motion — is not what shipped. The hub is *inert until
> touched*. That is a deliberate reversal, recorded in §1, and this philosophy
> section is left as written so the gap between the stated principle and the
> shipped behaviour stays visible.

---

## 1. Soap Bubble Device Hub

**Status: Shipped, drifted.**

### Original concept (from wireframe)

> "Deep black background with perfect white dot-grid. Big soap bubbles that can
> be played with, bounce around, and hit each other. Bubbles pop only when device
> disconnects."

The last sentence is the one that survived unchanged, and it is the load-bearing
part of the design.

### Background layer

**Drifted.**

| Specified | Shipped |
|---|---|
| Pure `#0a0a12` background | Themed. The canvas container is `background: transparent` (`DeviceHub.tsx:87`) and reveals the body-level `.starfield` layer (`globals.css:399-405`). |
| 2px circles at `rgba(255,255,255,0.04)`, 32px apart | A 26×26px SVG tile with a 1.1px-radius circle: white at 0.22 opacity in dark, `#1B2233` at 0.28 in light (`globals.css:182-183`, `:274-275`). Driven by the `--dot-grid` custom property. |
| Static texture, not interactive | Unchanged. `globals.css:924-925` explicitly pins the starfield so it "can never animate" — no twinkle. |
| Optional radial vignette | Not present. |

The reason for the reversal is recorded in the stylesheet: "the starfield read
as empty", and a uniform grid replaced it (`globals.css:390-395`). A pure-black
field with a near-invisible grid was, in the judgement recorded there, worse than
a visible one.

### Bubble physics

**Drifted — the largest single change in this document.**

| Specified | Shipped | Where |
|---|---|---|
| Each bubble "has an initial velocity and drifts slowly across the canvas" | **No ambient drift. Bubbles spawn at rest (`vx: 0, vy: 0`) and only ever move because the user throws them.** | `useBubblePhysics.ts:143-146` |
| "Damping: 0.998 per frame so bubbles gradually settle" | `DAMPING = 0.99`, applied as `Math.pow(DAMPING, dt)` so the decay rate is per-60fps-frame regardless of actual frame rate. | `useBubblePhysics.ts:41`, `:212-214` |
| "No friction — they coast forever unless they collide" | Opposite. Below `REST_SPEED = 0.05` a bubble is snapped to a full stop, and `MAX_SPEED = 4` caps a throw. | `useBubblePhysics.ts:44-47`, `:194-198` |
| "Idle drift: after 3 seconds of no interaction, bubbles resume gentle drift" | **Not implemented.** There is no idle timer and no resume behaviour. | — |
| 120–200px diameter, "varies by device importance" | 120–180px by device type (`desktop`/`laptop` 180, `phone` 160, `tablet`/`tv` 150, `watch`/`earbuds`/`headphones` 120, default 140). Not by importance. | `useBubblePhysics.ts:26-36` |
| Wall bouncing: "elastic collision" | Restitution 0.7 — visibly lossy, not elastic. | `useBubblePhysics.ts:205-208` |
| Bubble-to-bubble: elastic collision, mass = area | Implemented as specified, with a 0.9 elasticity factor and positional separation so bubbles never overlap. | `useBubblePhysics.ts:218-250` |
| Entry: "expands from 0 → full size with a spring (framer-motion)"; "enter from random edges and drift inward" | Scale animates 0 → 1 at 0.05/frame in the physics loop, driven by an inline transform. Not framer-motion, and new bubbles appear at a non-overlapping random position (`findOpenPosition`, 100 attempts, then a grid fallback) rather than entering from an edge. | `useBubblePhysics.ts:57-80`, `:182-184`; `DeviceBubble.tsx:367` |
| Pop: "scale to 1.2x over 150ms, burst into 6-8 particles, fade over 400ms, remove from DOM" | Implemented, and driven from two places. `useBubblePhysics` sets `popping`, eases scale to 1.2 and opacity to 0, and deletes the state after 600ms; `DeviceBubbleCanvas` emits a particle burst the moment a device leaves the list. 8 particles, drained one burst per 700ms. | `useBubblePhysics.ts:119-125`, `:175-179`; `DeviceBubbleCanvas.tsx:128-159`; `DeviceBubbleParticles.tsx:11` |
| Pop: device-coloured particles | Particles are emitted at a fixed grey `#8e8e93`, not the device's colour. | `DeviceBubbleCanvas.tsx:144` |
| Pop sound: "optional soft pop (TBD)" | Still TBD. Not built. | — |

**Why the drift.** The code explains itself: bubbles at rest "read as a static
grid", and a thrown bubble under 0.996 damping "creep[ed] across the hub for
~25s before the rest threshold could trigger" (`useBubblePhysics.ts:38-47`).
The hub reads as a place, and it responds when you touch it. The original
specification's ambient drift was, in practice, a screenshot of a screensaver.

**Not specified, and shipped:** `prefers-reduced-motion: reduce` is honoured.
It zeroes the speed ceiling (so velocity is zeroed immediately) and suppresses
the throw-on-release impulse, which guarantees stationary bubbles
(`useBubblePhysics.ts:98-109`, `:172`, `:304`). The bubbles are inert by default
in that mode, so the accessibility story is complete without the ambient motion
the spec assumed.

### Bubble appearance

**Drifted.**

The bubble is a frosted sphere defined entirely in CSS: `.glass-ball` sets the
border-radius and the drop shadow, `.glass-ball__body` supplies the gradient and
`backdrop-blur`, and `.glass-ball__ao` is an ambient-occlusion contact pool
behind the model (`globals.css:575-582`, `:622-666`). The design decision of
moving all of it out of inline styles and into classes is recorded at
`globals.css:584`.

| Specified | Shipped |
|---|---|
| Translucent circle, glass-like gradient at the device colour | Frosted orb, one soft highlight, one soft edge falloff, no caustics (`globals.css:621`). No per-device tint. |
| "Battery ring from current DeviceBubble (retained — it's great)" | **Dropped.** `DeviceBubble.tsx` renders no battery ring. `battery` is still passed down (`DeviceBubbleCanvas.tsx:189`) and `BubbleData` still carries it (`useBubblePhysics.ts:8`), but `DeviceBubble` does not use it. |
| Inner glow: radial gradient from center to transparent | Not present as a separate layer. |
| "Device name + status below icon (inside the bubble)" | Device name only, via `.glass-ball__label` with the font size driven by `--ball-radius` (`DeviceBubble.tsx:390`, `globals.css:688`). No status line. |
| "Status dot on the bottom-right edge of the bubble" | **Dropped.** |
| Device icon centered inside | Replaced by a suspended 3D device model — CSS 3D-transformed phone, desktop, tablet, watch, headphones, TV and earbuds models (`DeviceBubble.tsx:22-326`). Sized at `radius × 1.10` to keep the model inside the sphere. |

### Interaction

**Shipped, drifted on one threshold.**

| Interaction | Specified | Shipped |
|---|---|---|
| Hover | Scale 1.05, glow intensifies | `scale(1.045)` and a deeper shadow (`globals.css:591-593`, `:604-608`). |
| Click | Opens `DeviceDetailPanel` | Unchanged. Also opens the detail panel on Shift+click? No — Shift+click starts **audio streaming** to that device (`DeviceBubbleCanvas.tsx:103-111`, `DeviceHub.tsx:97`). Not in the spec. |
| Drag | "Grab a bubble and fling it — it inherits your velocity and bounces around" | Implemented, with fling velocity from the last four pointer samples, scaled by 0.35. Drag is handled on the *canvas*, not on the bubble, and a >4px move is required to distinguish a drag from a click. Grabbing a bubble cancels any residual motion, so the throw on release is the only thing that can set velocity. |
| Selected state | not specified | `scale(1.06)` plus an accent glow (`globals.css:594-597`, `:609-614`). |
| Dragging state | not specified | `scale(0.97)` and a tighter shadow (`globals.css:598-601`, `:615-619`). |

### Layout modes

**Not implemented.** Free mode is the only mode. `DeviceHub.tsx` holds one piece
of state — `detailDevice` — and has no mode toggle; the header contains only the
title, a status badge and the Pair button (`DeviceHub.tsx:100-128`). Orbit mode
and the mode-switch button described in the spec do not exist. Tracked in
the project Limitations section.

### Implementation — actual file layout

The spec's file list is wrong; the components live under `components/network/`,
and there is no `DotGrid.tsx` and no `tailwind.config.js` in this project (the
theme is CSS custom properties plus utility classes used directly).

| Specified | Actual |
|---|---|
| `src/components/Devicehub/SoapBubbleCanvas.tsx` | `src/components/network/DeviceBubbleCanvas.tsx` |
| `src/components/Devicehub/SoapBubble.tsx` | `src/components/network/DeviceBubble.tsx` |
| `src/components/Devicehub/DotGrid.tsx` | Does not exist; the grid is the `--dot-grid` custom property on `.starfield` |
| `src/components/Devicehub/BubbleParticles.tsx` | `src/components/network/DeviceBubbleParticles.tsx` |
| `src/hooks/useBubblePhysics.ts` | Unchanged — correct |
| `DeviceHub.tsx` — replace static grid with the canvas | Done: `DeviceHub.tsx:89-98` |
| `tailwind.config.js` — add pop keyframes | Not applicable; the project has no `tailwind.config.js` |

External dependencies: `motion/react` for the pop and entry animations only. The
physics loop is plain `requestAnimationFrame`, as specified.

---

## 2. Swipe Gestures for Notifications & Calls

**Status: Shipped for notifications, partially shipped for calls.**

### Original concept (from wireframe)

> Notifications: "drag left to mark as read" / "drag right to dismiss"
> Calls: "drag left to cut call" / "drag right to accept call"

### Notification swipe

**Shipped.**

`NotificationCard` is wrapped in `SwipeableCard` (`NotificationCard.tsx:98-100`,
closing at `:222`). Left is Mark Read, right is Dismiss, matching the wireframe.

| Specified | Shipped | Where |
|---|---|---|
| Threshold ≥80px | `threshold = 80` default, **or** a fling with \|velocity\| > 500 px/s | `SwipeableCard.tsx:24`, `:53-56` |
| Left reveals a green "Mark Read" indicator, right a red "Dismiss" indicator | Implemented. Default labels are `'Mark Read'` and `'Dismiss'`; colours default to `rgba(16,185,129,0.15)` and `rgba(248,113,113,0.15)`. | `SwipeableCard.tsx:25-28` |
| Indicator opacity scales proportionally with drag distance | Implemented with `useTransform` and `clamp: true`. | `SwipeableCard.tsx:38-41` |
| Insufficient drag springs back | Spring `stiffness: 500, damping: 30`. | `SwipeableCard.tsx:58-62` |
| "Card slides out left/right, fades" | The card animates to ±400px over 250ms, then the handler runs. | `SwipeableCard.tsx:54-56` |
| "Subtle arrow/label that appears when drag is near the threshold" | Partially: the label fades in with the drag, with no separate near-threshold affordance. | `SwipeableCard.tsx:75-94` |
| — | Not in the spec: the card also scales to 0.97 at ±200px of drag. | `SwipeableCard.tsx:44` |

When `onMarkRead` is not supplied, swipe-left falls back to Dismiss
(`NotificationCard.tsx:42-48`), so a card is never left with a dead gesture.

**The spec's file list is wrong.** `src/hooks/useSwipeGesture.ts` does not
exist; the component uses `motion/react`'s `drag` handler directly, which is
simpler than a hand-rolled pointer-event tracker. `SwipeableCard.tsx` is at
`src/components/ui/SwipeableCard.tsx`, as specified. `IncomingCall.tsx` was
specified to be wrapped in `SwipeableCard` — **it is not.**

### Call swipe

**Partially shipped. Only the floating incoming-call overlay is swipeable.**

`IncomingCall` implements its own `motion` drag rather than using
`SwipeableCard` (`IncomingCall.tsx:216-218`), with its own motion values, its own
thresholds and its own visual language (a green/red full-bleed wash and a rotate
transform driven by `x`, `IncomingCall.tsx:55-59`).

| Specified | Shipped | Where |
|---|---|---|
| The incoming call card is draggable **vertically and horizontally** | **Horizontal only** (`drag="x"`, constraints `{left: 0, right: 0}`). | `IncomingCall.tsx:216-217` |
| Swipe right ≥100px → accept | ≥**120px**, or velocity > 800 px/s. | `IncomingCall.tsx:117-118` |
| Swipe left ≥100px → decline | ≥**120px**, or velocity < −800 px/s. | `IncomingCall.tsx:119-120` |
| **Swipe down ≥120px → dismiss the overlay** | **Not implemented.** There is no vertical drag axis. | — |
| Right drag shows green glow + phone icon; left shows red + phone-off | Implemented as a full-bleed background wash whose opacity scales with `x`, over a 120px ramp. | `IncomingCall.tsx:58-59`, `:182-200` |
| Keyboard shortcuts retained | Implemented, and extended: `Shift+ArrowLeft` rejects and `Shift+ArrowRight` accepts. | `IncomingCall.tsx:98-111` |
| — | Not in the spec: two-finger **trackpad** swipe. `onWheel` accumulates horizontal deltas and fires at ±80px after a 100ms debounce, gated on `status === 'ringing'`. | `IncomingCall.tsx:68-80`, `:167` |

The spec's "swipe is an enhancement, not a replacement" holds: the Answer and
Reject buttons are still there and still work.

### Call-history rows are not swipeable — and a hint implies they are

`CallHistory.tsx` contains no drag, swipe or motion handler at all.

The copy in `CallsNotificationsSplit.tsx:109-130` is **wordingally correct** — the
heading is "Call popup gestures", the rows read "Swipe left to reject" / "Swipe
right to accept", and the footnote says "Drag the call popup or use a two-finger
trackpad swipe". Every one of those statements is true of `IncomingCall`.

The problem is placement: the hint panel is rendered **immediately below the
call-history list** (after `CallHistory` at `CallsNotificationsSplit.tsx:100-107`),
inside the same column, with no visual boundary. A user reading top to bottom
sees a list of rows, then a gesture legend, and the reasonable conclusion is
that the rows are swipeable. They are not.

**Design intent: the gesture legend belongs with the popup it describes.** Either
move it into `IncomingCall` (where the gestures are actually available) or give
it an explicit heading that names the floating overlay, as the current one does
but does not make legible in context. Tracked in
the project Limitations section.

---

## 3. Adaptive Masonry Grid (Clipboard & Files)

**Status: Shipped, restructured.**

### Original concept (from wireframe)

> "Not a fixed grid, but size depends on the text and file type"

This is the one feature where the original intent was kept essentially intact —
and where the implementation went further than the spec by **merging** the two
lists the spec treated separately.

### Layout

| Specified | Shipped |
|---|---|
| CSS columns-based masonry, no library | Unchanged. `columns-2 gap-4` (`MergedMasonry.tsx:142`). |
| 3 columns wide / 2 medium / 1 narrow | **2 columns, no responsive breakpoint.** The `xl:columns-3` variant the code's own doc comment describes is not applied. |
| Gap: 12px uniform | 16px between columns (`gap-4`), 12px between cards (`mb-3` on each card). |
| Cards flow top-to-bottom, left-to-right | Unchanged — that is how CSS columns work. |

Cards are content-height with `break-inside-avoid` (`ClipboardCard.tsx:133`,
`FileCard.tsx:111`), which is the mechanism the "size depends on the text and
file type" requirement needs. Nothing is put on a fixed row height.

### The merge

The spec describes two views — a masonry clipboard view and a masonry file view —
plus a toggle between a list and a grid. **The shipped design has one surface.**
`MergedMasonry` takes transfers and clipboard items, merges them into a single
stream sorted by `timestamp` descending, and renders the result in one masonry
(`MergedMasonry.tsx:78-109`). `FilesScreen` is the only host
(`FilesScreen.tsx:69`).

The reasoning is in the component: "no column reads as 'the files list' or 'the
clipboard list'" (`MergedMasonry.tsx:58-59`), and "Files + Clipboard in ONE
frosted panel — no tabs, no split lists" (`FilesScreen.tsx:11`). Empty states
ride the same masonry as placeholder cards rather than occupying a second section
(`MergedMasonry.tsx:111-137`).

**There is no list/grid toggle.** The masonry is the only view.

### Clipboard cards

| Specified | Shipped |
|---|---|
| Height scales with content length; <50 / 50-200 / >200 char tiers with 1/3/5-line previews | Content-height, but the tiers are gone. Line cap is computed per type: `2` for links, `min(12, …)` for code, `min(10, …)` for text, floored at 3. | `ClipboardCard.tsx:122` |
| "> 200 chars: tall card, 5-line preview + 'show more' expand" | **No expand exists.** Truncated content gets a trailing `…` marker and nothing else — there is no way to see the rest in place. | `ClipboardCard.tsx:188-190`, `:196-198` |
| Image clips: fixed 16:9, thumbnail preview | Typed as an image clip for the chip, but it falls through to the **text** preview branch — an image clip renders its content as text. No image rendering. | `ClipboardCard.tsx:193-199` |
| Link clips: title + domain favicon + URL preview | URL plus extracted **domain**, truncated. No favicon fetch. | `ClipboardCard.tsx:163-171` |
| Code clips: monospace, syntax-highlighted, max 8 lines with scroll | Monospace, up to 12 lines, `overflow-hidden` — no scroll, no syntax highlighting. | `ClipboardCard.tsx:184-186` |
| Each card shows content preview, source device icon, timestamp | Preview and type chip are there. The type taxonomy is richer than specified: `link`, `code`, `text`, `image`, `ssh`, `coordinates` (`ClipboardCard.tsx:38-43`). |
| Click to copy again; hover shows copy / pin / delete | Handlers are plumbed through `MergedMasonry` as `onCopy` / `onPin` / `onDelete` (`MergedMasonry.tsx:35-37`). |

### File transfer cards

| Specified | Shipped |
|---|---|
| Image files: thumbnail; documents: icon + filename + size | Unchanged in structure, but the "thumbnail" is a `FileIcon` glyph in a `aspect-[4/3]` box — no image data is loaded. Video gets the same treatment in an `aspect-video` box. | `FileCard.tsx:124-131` |
| Audio files: waveform visualisation (simple bar chart) | A fixed 12-bar decorative strip with hard-coded heights. Not a waveform of the audio. | `FileCard.tsx:133-137` |
| Active transfers: **animated progress ring around the file icon** | A linear progress bar under the metadata. Not a ring. | `FileCard.tsx:159` |
| Completed transfers: subtle checkmark badge | Not present as a distinct badge; the status text carries it. |
| Hover: lift (translateY −2px) + border glow | The card has `transition-all` and a `group` hover class (`FileCard.tsx:111`). The exact lift was not re-verified. |
| Click: expand to full preview (modal overlay) | Clicking opens the saved path (`onOpen`), not a modal preview. |
| "Drag: Reorder is NOT implemented (future feature)" | Confirmed still not implemented. This is the one "Not implemented" in §3 that the spec itself already declared. |

### Implementation — actual file layout

| Specified | Actual |
|---|---|
| `src/components/clipboard/ClipboardHistory.tsx` | Does not exist. The clipboard half of the masonry is `MergedMasonry.tsx`. |
| `src/components/clipboard/ClipboardCard.tsx` | Unchanged — correct |
| `src/components/files/FileCard.tsx` | Unchanged — correct |
| `src/hooks/useClipboardHistory.ts` | Does not exist. Clipboard state comes from `useClipboardState` / `useClipboard`. |
| `FileExplorer.tsx` — add a list/grid toggle | Does not exist. The host is `FilesScreen.tsx`. No toggle was added. |
| `Sidebar.tsx` — add a "Clipboard" nav item | Does not exist. Navigation is `NavigationContext` / `useAppNavigation`; clipboard and files share one destination. |
| `App.tsx` — add clipboard history state + route to new view | Clipboard state is wired; there is no separate clipboard route. |
| `tailwind.config.js` — masonry utilities | Not applicable; the project has no `tailwind.config.js`. |

External dependencies: none, as specified.

---

## 4. Unified Transition System

**Status: Shipped, drifted — no standard was ever specified numerically.**

The spec calls for a consistent view transition: content fades and slides up
16px over 250ms, children staggered 30ms apart, via `AnimatePresence` +
`motion.div`.

`AnimatePresence` and `motion` are used in `App.tsx`, and the page transition is
a single `PAGE_VARIANTS` object — so the "one transition system" half of the
intent holds. But the shipped values are **6px, not 16px, and 180ms, not
250ms** (`App.tsx:273-277`: `initial: { opacity: 0, y: 6 }`, `duration: 0.18`,
`easeOut`, exit `duration: 0.1`). No staggered-child delay is implemented
anywhere.

The direction (one transition system via `motion`) holds. The numbers in this
section were never turned into a shared constant, so they are not enforced. If
the 250ms/16px/30ms values are the intent rather than the sketch, they need to
be extracted into a token and applied in one place; that is open work in
the project Limitations section.

## 5. Micro-interactions

**Status: Shipped, drifted.**

| Specified | Shipped |
|---|---|
| Button hover: scale 1.02 + border glow | Not implemented as a scale. Pressed state is a `.btn-press` utility (`globals.css:890-893`); hover is a background-wash utility. |
| Active/pressed: scale 0.98 | Not implemented as a scale. |
| Toggle switches: spring animation | Not verified. |
| Badge counts: number flip animation on change | **Not implemented as specified.** The dock badge has a spring pop-in (`initial={{ scale: 0 }}` → `animate={{ scale: 1 }}`, `stiffness: 600, damping: 20`) but it is a *mount* animation, not a value-change animation — incrementing 2 → 3 does not animate, and the badge does not re-key. `FloatingDock.tsx:169-193`. |

**Not implemented:** the number-flip on value change, and the hover/active
scale figures (1.02 / 0.98) — pressed state is the `.btn-press` utility
(`globals.css:890-893`) and hover is a background wash
(`FloatingDock.tsx:66`, `:76`), not a transform. Everything else in this section
is present in some form but not to the numbers written here.

## 6. Dark Mode Polish

**Status: Shipped, drifted — the token names in this section are all wrong.**

The design language shipped; the vocabulary did not. This project uses CSS
custom properties in `globals.css`, with theme-specific overrides, not the
Tailwind `bg-primary` / `secondary` / `tertiary` names this section specifies.

| Specified | Shipped |
|---|---|
| `bg-primary` / `secondary` / `tertiary` | `--bg-0` / `--bg-1` / `--bg-2` |
| `backdrop-blur-glass` + `surface-glass` | `.surf-frost`, `.surf-clear`, `.glass-ball__body` |
| `border-border` (6% white) + `border-subtle` (3% white) | `--line-1` / `--line-2` |
| text tokens | `--ink-1` / `--ink-2` / `--ink-3`, `--text-1` / `--text-2` |

There is a documented rule the spec does not mention and the code enforces
strictly: "never text tokens inside `.surf-frost` · never ink tokens on
starfield" (`globals.css:501`), because the accent collapses onto a bright
frosted surface. Light and dark themes are both first-class, with per-theme dot
grid and shadow values, not a dark theme with a light override.

---

## 7. Not implemented — items outside Features 1–3

These were verified against the source on 2026-09. They are listed here because
they are the UI's most load-bearing *unbuilt* affordances, and because each one
currently presents as though it works.

### 7.1 Custom title bar — not implemented

`src/components/layout/TitleBar.tsx` exists and is complete. **It is imported
by nothing** — not by `App.tsx`, not by any view, not by any test. The app uses
the OS title bar: `tauri.conf.json:20` sets `"decorations": true`.

There is no design intent for a custom title bar in this document, so this is
not a design failure — it is a dead component. It should either be adopted
(which would require `decorations: false` and a drag-region implementation) or
deleted. Tracked in the project Limitations section.

### 7.2 Interactive tutorial — unreachable

`src/components/ui/InteractiveTutorial.tsx` is rendered at `App.tsx:406` when
`shell.showTutorial` is true. **`showTutorial` can never be true.**
`useAppShell` initialises it to `false` (`useAppShell.ts:18`) and the only
setter, `setShowTutorial` (`useAppShell.ts:48-50`), is exported and never
called. The component is unreachable.

The Settings entry labelled **"Show tutorial again"**
(`Settings.tsx:175-188`) does not open the tutorial. It removes
`conduit_onboarded` from `localStorage` and reloads the page
(`Settings.tsx:179-182`), which re-shows **`Onboarding`**, a different
component. The button is labelled for a feature it does not open, and the
tutorial it names is not the thing the user sees.

**Design intent retained:** the tutorial component's own design is fine; it is
the wiring that is missing. One of two things has to happen — wire
`setShowTutorial` to a trigger, or relabel the Settings button to match what it
actually does ("Show welcome again"). Tracked in
the project Limitations section.

### 7.3 Dock "Surround Sound" button — never renders

`FloatingDock` renders the Surround Sound button only when
`onToggleSurroundSound` is passed (`FloatingDock.tsx:219`). `App.tsx:450-456`
renders the dock without it and without `surroundSoundActive`.

The state exists and works: `useCalls` owns `surroundSoundActive` and returns it
(`useCalls.ts:54`, `:227`). The dock is simply not given it. The button is dead
UI with no trigger. Tracked in
the project Limitations section.

### 7.4 Dock "hover to expand secondary actions" — deliberately dropped

The dock is **explicitly non-expanding**. The component's own header comment
states it: "Revision 3: ONE non-expanding dock row — no flyout tray. Every
destination [is] in the row … Settings, then the optional Surround toggle and
the Pair '+' button last" (`FloatingDock.tsx:43-45`).

Hover only applies a background wash (`FloatingDock.tsx:66`, `:76`) and reveals
a tooltip (`FloatingDock.tsx:83`). There is no expansion, no secondary tray and
no flyout. This is a **Dropped**, not a gap — the reversal is deliberate and
documented in the code, and it should not be re-proposed without a reason.

### 7.5 Settings "Storage Usage" — hardcoded

`AdvancedSection.tsx:178-184` renders a `Storage Usage` row reading
**`~2.1 MB`**. The value is a string literal. There is no Tauri command that
reports a database or cache size, and no measurement of any kind behind the
number. The sibling "Clear Cache" button is real
(`AdvancedSection.tsx:186-196`); the number above it is not.

This is a live false claim in shipped UI: it will read as authoritative and will
be wrong for every user. Either measure it (a `storage_usage` command returning
`fs::metadata(...).len()` for the database and its sidecars) or remove the
number. Tracked in the project Limitations section.

---

## Implementation Order (original, and where each phase landed)

| Phase | Feature | Original estimate | Status |
|---|---|---|---|
| 1 | Soap Bubble Device Hub | 3–4 hrs | **Shipped, drifted** — no ambient drift, orbit mode not built |
| 2 | Swipe Gestures (Notifications + Calls) | 2–3 hrs | **Shipped for notifications, partial for calls** — no vertical drag, no swipe-down |
| 3 | Adaptive Masonry Grid (Clipboard + Files) | 2–3 hrs | **Shipped, restructured** — one merged surface, no list/grid toggle, 2 columns |
| 4 | Transition system + micro-interactions | 1 hr | **Shipped, drifted** — no shared transition token, numbers not enforced |

Phases 1–3 were treated as independent. That judgement held.
