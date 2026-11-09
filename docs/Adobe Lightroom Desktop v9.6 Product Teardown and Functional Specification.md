# Adobe Lightroom Desktop v9.6: Product Teardown and Functional Specification

Prepared for Tyler | Research date: October 31, 2026 | Evidence: 21 official Adobe Lightroom Desktop help pages opened in this session

## 0. Read this first

### 0.1 Scope guard

This document specifies only the cloud-centric Adobe Lightroom desktop app for Windows and macOS. Lightroom Classic, Mobile, Web and standalone Photoshop are excluded. Adobe keeps a separate help tree for each surface, so only the Desktop tree (helpx.adobe.com/lightroom/desktop) counts as evidence. Classic, Web and Mobile pages that surfaced during searching were discarded, including a Web-only shortcut list (single keys such as P and Shift+?) that does not describe the desktop app. The brief skips sections 7 and 11 (numbering jumps 6 to 8 and 10 to 12); I treated that as intentional and added nothing for them.

### 0.2 Bottom line: findings that shape an architecture

1. **Two libraries with different rules.** The Cloud tab is a managed library: Add Photos copies files in and uploads full-resolution originals; albums, sharing, People, Improved Search and stacks are cloud features. The Local tab edits files in place on disk with no import; edits are stored non-destructively inside the file or in an XMP sidecar, and Copy to Cloud creates an independent cloud copy. Do not assume feature parity between the two. \[S7 S8 S9 S17 V\]
2. **Editing is gated on Detail view.** Photo Grid and Square Grid are browse and organize only. \[S8 V\]
3. **No documented History panel.** The snapshot mechanism is Versions (an Auto tab plus a Named tab). Treat Classic-style History or Snapshots as absent until hands-on confirmation. \[S3 V; absence H\]
4. **Enhancement and generative tools mostly emit new files, not parameters.** Denoise, Raw Details and Super Resolution write new DNGs; Upscale, AI Sharpen, Generate Video and Prompt to Edit write a new image or video beside, or stacked with, the original. Stacks are documented as Cloud-only. \[S13 S14 S16 S17 V\]
5. **Local adjustments are mask-first.** Brush, gradients, AI selections and range masks feed one mask panel with a shared slider set; Remove, Clone and Heal are separate tools. \[S4 S12 V\]
6. **AI is metered and partner-powered.** Upscale and AI Sharpen cost 10 credits up to 25 MP and 20 credits for 25-56 MP; Firefly Fill & Expand, Generate Video and Prompt to Edit are credit-gated; Content-Aware Fill is free and Generative Remove was documented as free in Oct 2025. Partner models surface in the UI (Topaz, Google Veo 3.1, Nano Banana 2). \[S12 S13 S15 S16 S17 V\]
7. **GPU is a hard dependency for heavy features.** Full acceleration plus Denoise, Lens Blur and Reflection Removal call for 8 GB dedicated (or 16 GB shared) GPU memory on Windows and Apple silicon with 16 GB unified memory on macOS. \[S21 V\]
8. **Documentation is uneven.** Twelve conflicts are logged in 0.6 and several pages predate v9.6.

### 0.3 Current version and release timeline

| Version | Released | Added on Desktop | Src |
| --- | --- | --- | --- |
| 9.6 | Sep 2026 | Prompt to Edit (Adobe labels it early access); improved Reflection Removal; new camera and lens profiles | S2 S17 |
| 9.5 | Aug 2026 | Generative Expand; Feather and Edge sliders for masks | S2 S4 S15 |
| 9.4 | Jun 2026 | Faces panel in Assisted Culling; AI Sharpen; Generate Video; more precise Select Subject; custom color labels; drag-to-edit tone in the Histogram | S2 |
| 9.3 | Apr 2026 | Natural-language search; send photos to Firefly boards; Film-Inspired presets; pan and zoom while cropping; PSB files; Generative Upscale (Topaz); Edit in Firefly; WebP | S2 S10 |
| 9.0 to 9.2 | from Oct 2025 | Assisted Culling; dust-spot removal; Color Variance slider; snow in Select Landscape plus Landscape Adaptive presets; Color Labels; Local-tab subfolders; batch-rename templates | S2 S21 |
| Earlier | 2024 to mid 2025 | Adaptive Profiles (2024, extended for HDR in 2025); Select Landscape; People Removal and Reflection Removal | S2 |

S2 stamps versions 9.3 to 9.6 only. Version 9.0 equals the October 2025 release per S21. Timing of the 9.0-9.2 and earlier rows is inferred from S2 ordering and image names (H).

### 0.4 Verification legend

| Tag | Meaning |
| --- | --- |
| V | Verified: stated on an Adobe Lightroom Desktop help page opened in this session (last-updated dates in 0.5) |
| I | Index-only: the feature or page exists in Adobe's Desktop help index or What's New, but its body was not opened; no details are asserted |
| C | Conflict: Adobe's own pages disagree (see 0.6) |
| H | Hands-on needed: docs are silent or ambiguous, or the line comes from general product convention and must be confirmed in the running app |

Availability labels used in the feature matrix: Available; Limited availability (Adobe says early access); Technology Preview; Announced, not generally available; Unverified.

### 0.5 Source key

| ID | Adobe page | Last updated | URL |
| --- | --- | --- | --- |
| S1 | Lightroom Desktop Help index | not recorded | https://helpx.adobe.com/lightroom/desktop.html |
| S2 | What's new in Lightroom | Sep 29, 2026 | https://helpx.adobe.com/lightroom/desktop/introduction/whats-new.html |
| S3 | Edit photos | Oct 18, 2026 | https://helpx.adobe.com/lightroom/desktop/edit-photos/edit-photos.html |
| S4 | Apply Masking for local adjustments | not recorded | https://helpx.adobe.com/lightroom/desktop/edit-photos/masking.html |
| S5 | Set preferences | Jan 23, 2025 | https://helpx.adobe.com/lightroom/desktop/introduction/preferences.html |
| S6 | Export or share your photos | Apr 24, 2025 | https://helpx.adobe.com/lightroom/desktop/save-share-and-export/save-share-photos.html |
| S7 | Import and access photos (Cloud, Local, archive) | Mar 27, 2026 | https://helpx.adobe.com/lightroom/desktop/add-import-and-capture-photos/access-photos.html |
| S8 | Organize photos | Apr 16, 2026 | https://helpx.adobe.com/lightroom/desktop/organize-photos/organize-photos.html |
| S9 | Add photos | Apr 24, 2025 | https://helpx.adobe.com/lightroom/desktop/add-import-and-capture-photos/add-photos.html |
| S10 | Edit photos with Presets | Apr 16, 2026 | https://helpx.adobe.com/lightroom/desktop/edit-photos/presets.html |
| S11 | Crop, Rotate, and Geometry | Apr 16, 2026 | https://helpx.adobe.com/lightroom/desktop/edit-photos/crop-rotate-geometry.html |
| S12 | Generative Remove | Oct 27, 2025 | https://helpx.adobe.com/lightroom/desktop/edit-photos/remove-tool.html |
| S13 | Enhance photos using generative AI (Upscale, AI Sharpen) | Jun 18, 2026 | https://helpx.adobe.com/lightroom/desktop/edit-photos/enhance-images-with-generative-ai.html |
| S14 | Enhance image quality (Denoise, Raw Details, Super Resolution) | Jun 17, 2025 | https://helpx.adobe.com/lightroom/desktop/edit-photos/enhance-details.html |
| S15 | Generative Expand | Aug 13, 2026 | https://helpx.adobe.com/lightroom/desktop/edit-photos/extend-photos-with-generative-expand.html |
| S16 | Generate videos from images | Jun 18, 2026 | https://helpx.adobe.com/lightroom/desktop/edit-photos/generate-videos-from-images.html |
| S17 | Edit photos using prompts (Prompt to Edit) | Sep 29, 2026 | https://helpx.adobe.com/lightroom/desktop/edit-photos/edit-photos-using-prompts.html |
| S18 | Assisted Culling | Jun 18, 2026 | https://helpx.adobe.com/lightroom/desktop/organize-photos/assisted-culling.html |
| S19 | People View | Aug 28, 2025 | https://helpx.adobe.com/lightroom/desktop/organize-photos/people-view.html |
| S20 | Default keyboard shortcuts | Jun 17, 2025 | https://helpx.adobe.com/lightroom/desktop/using/keyboard-shortcuts.html |
| S21 | System requirements | Oct 12, 2026 | https://helpx.adobe.com/lightroom/desktop/introduction/system-requirements.html |

### 0.6 Conflicts and gaps inside Adobe's own documentation

1. **Export formats (S6).** The page intro lists JPG, AVIF, JPEG XL, DNG, TIF and Original; the Custom Settings description lists only JPG, TIF, DNG and Original. AVIF and JPEG XL export are H. \[C\]
2. **Denoise formats (S14).** Foveon and Pentax PSR appear under both supported and unsupported lists, and the opening paragraph names a much narrower set (Bayer, X-Trans, Linear DNG, Apple ProRAW). \[C\]
3. **Enhance shortcut and OS floor (S14).** Batch Denoise says Cmd+Option+E on macOS while the Enhance section says Control+Option+E; the page also states macOS 10.14 and Windows 10 1903 minimums that contradict S21. \[C\]
4. **Brush size keys (S20).** The left bracket is published as increase and the right bracket as decrease, the reverse of common convention. \[C/H\]
5. **Shortcut collisions (S20).** Cmd/Ctrl+H is listed for both Hide Lightroom and Merge to HDR; Cmd/Ctrl+M for both Minimize and Merge to Pano; Paste Selected Settings shows Shift+Ctrl+V for macOS too; Rotate Right is mislabeled CCW. \[C\]
6. **Naming drift.** Detail view, Loupe view and Edit view describe the same editing surface (S3 S4 S12); the left browser is called All Photos in S8 and My Photos in S3 and S19. \[C\]
7. **Staleness.** Preferences (Jan 2025), Shortcuts (Jun 2025), Export (Apr 2025) and Add photos (Apr 2025) predate v9.3-9.6; there are no documented shortcuts for Prompt to Edit, Generative Expand or Generate Video. \[H\]
8. **Improved Search (S8).** The page says the Info icon appears when 'Lightroom on the web' finishes indexing; whether desktop indexes locally, in the cloud, or covers Local-tab photos is unspecified. \[H\]
9. **Credits policy (S12 vs S13 S15 S16 S17).** Generative Remove is documented as not deducting credits (Oct 2025) while newer AI tools do. Re-verify before copying a pricing model. \[H\]
10. **Stacks (S13 S16 S17).** Stacks are described as Cloud-only; Local behavior is unspecified although Group Into Stack shortcuts exist (S20). \[H\]
11. **Create Preset (S10).** Documented as a name-only dialog; no per-setting inclusion UI is described. Do not assume Classic's checkbox list. \[H\]
12. **Prompt to Edit status.** S17 calls it early access while S2 presents it as a normal 9.6 feature; classify as Limited availability. \[C\]

### 0.7 Coverage scorecard and pages not opened

| Brief section | Coverage this pass |
| --- | --- |
| 1 Structure and navigation | Good for panes, views, panels, dialogs; Window and Help menus and exact toolbar layout missing |
| 2 Component inventory | Good for Edit sections, masks, Presets, Export, Culling, People; thin on hover/disabled/error states |
| 3.1-3.8 Edit panels | Good on control names; numeric ranges only where Adobe states them |
| 3.9 Remove and retouch | Generative Remove good; Clone, Heal, People, Reflection, Dust index-only |
| 3.10-3.11 Presets and utilities | Good; compare modes and batch sync partial |
| 4 Library | Good |
| 5 Import, Local, Cloud | Good; sync conflict and error flows undocumented |
| 6 Export and share | Good (Apr 2025 page) |
| 8 Menus, shortcuts, prefs | Shortcuts complete per S20; prefs from a Jan 2025 page |
| 9 AI | Good; statuses assigned |
| 10 Interaction | Partial; many items H |

Index-only pages (details not asserted): Clone tool, Remove people, Remove reflections, Remove dust, Merge HDR/Pano, HDR Optimization, Edit in other apps, Content Credentials, Edit Videos, Stacks, Rename files, Color labels, Release notes, FAQ, In-app learning, Lens Blur, Heal tool, Generative Remove FAQ, Firefly boards, Edit in Firefly, Topaz and generative-credit FAQs, GPU FAQ, free/starter export and premium-features pages, Apple TV setup, mobile preset sync, migration guides (Apple Photos, Classic), Known and Fixed issues.

## A. Desktop application map

Tags in brackets are source plus status (see 0.4 and 0.5). H items are assumptions to confirm hands-on.

### A.1 Window structure

- **Application window** (Windows 10 22H2 or 11 23H2+; macOS 14+) \[S21 V\]
  - **OS menu bar**
    - File: Add Photos... (Shift+Cmd/Ctrl+I), Export (Shift+E), Export with Previous (Cmd/Ctrl+E) \[S6 S9 S20 V\]; other File items H
    - Edit: Stacks > Group Into Stacks / Ungroup Stack; Preferences on Windows (Edit > Preferences, Ctrl+,) \[S8 S19 S20 V\]; Undo, Redo, Cut, Copy, Paste, Select All, Select None exist as shortcuts, menu placement H
    - Photo: Copy Edit Settings, Choose Edit Settings to Copy, Paste Edit Settings, Reset To Original, Reset To Open \[S3 V\]; Enhance (Denoise, Raw Details, Super Resolution) \[S14 V\]; Enhance with AI > Generative Upscale, AI Sharpen \[S13 V\]; Generate Video \[S16 V\]
    - View: Detail - Full Screen, Keywords, Prompt to Edit \[S8 S17 V\]; further items H
    - Window and Help menus: not documented on any opened page (H)
    - macOS application menu 'Adobe Lightroom': Preferences (Cmd+,), Hide, Hide Others, Quit \[S19 S20 V\]
  - **Header bar**
    - Add Photos button, upper-left \[S9 V\]
    - Job-progress indicator, upper-left of the header, for Upscale, AI Sharpen, Generate Video, Prompt to Edit \[S13 S16 S17 V\]
    - Search bar, Filter menu, Sort menu \[S8 V\]
    - Share icon, upper-right: JPG (Small), JPG (Large), Original, Previous Settings, Custom Settings \[S6 V\]
    - Grid-header actions: Share & Invite icon, three-dot menu \[S6 V\]
  - **Left pane: Photos browser** (opened with the My Photos icon) \[S3 S19 V\]
    - Tab **Cloud**
      - Add Photos; Learn (tutorials, Lightroom Academy); Community (My Profile, Featured, Following, Remixes) \[S7 V\]
      - All Photos: Recently Added, Recent Edits, By Date, By People (needs Enable People View), Connections, Deleted \[S7 S8 V\]
      - Albums: Create Album, Create Smart Album, Create Folder; sort by Album Name, Recently Updated, Date Created, Photo Count; display Name & Covers or Name Only \[S7 S8 V\]
      - Shared: Shared with You, Shared Albums, Shared Photos; Shared to Web panel \[S6 S8 V\]
      - Sharing panel: Connections (Adobe Portfolio, Blurb, WHCC) \[S6 V\]
    - Tab **Local**: Favorites, Browse (all folders and connected drives), Include subfolders switch, folder More menu \[S7 V\]
    - **People view** (My Photos > People, opt-in) \[S19 V\]
    - Discover panel (in-app help, tips, learning) \[S11 I\]
  - **Main content area**
    - Photo Grid, Square Grid, Detail (the only view with active edit controls) \[S8 V\]
    - Filmstrip (toggle with the slash key) \[S20 V\]
    - Full-screen mode, reachable only from Detail; F or Esc exits \[S8 V\]
    - Review-for-import screen \[S9 V\]; Assisted Culling workspace \[S18 V\]; People cluster pages \[S19 V\]
  - **Right-hand panels in Detail view**
    - Info (I): metadata, Sync Status, Captured date editor, Culling Info \[S8 S18 V\]
    - Keywords (K): keywords and People names \[S8 S19 V\]
    - Versions (Shift+V): Auto tab and Named tab \[S3 S20 V\]
    - Presets (Shift+P): Recommended, Premium, Yours \[S10 V\]
    - Edit (E): Histogram and the sections below \[S3 V\]
  - **Tool launchers**: Crop/Rotate/Geometry (C) \[S11 S20 V\]; Heal/Remove (H) \[S12 S20 V\]; mask tools on the right-hand toolbar \[S4 V\]; Prompt to Edit on the left panel \[S17 V\]; exact rail placement of each is H
  - **Edit panel sections** (Cmd/Ctrl+1 to 6; Cmd/Ctrl+0 toggles the histogram)
    - Histogram (three-dot menu 'Show Histogram'; J toggles clipping overlays) \[S3 V\]
    - Profile browser \[S3 V\]; Light (1) \[S3 V\]; Color (2) \[S3 V\]; Effects (3) \[S3 V\]; Detail (4) \[S3 S14 V\]; Optics (5) \[S3 V\]; Geometry (6) with the Crop panel and Generative Expand \[S3 S11 S15 V\]
  - **Preferences dialog**: Account, Cache, General, Import, Interface, Performance, Technology Preview \[S5 V\]

### A.2 Dialog and popover inventory

- Photos will be Synced to Creative Cloud confirmation \[S7 V\]
- Review for Import screen: hover selection circle, Select All, 'Previously Added' overlay, Add To Album, Add Photos \[S9 V\]
- Add Photo(s) dialog for shared albums: include keywords, ratings and flags, GPS \[S6 V\]
- Archive to Local Storage: Save to, Organize (one folder or by date), Existing Files (keep both or replace) \[S7 V\]
- Create Smart Album: Title, Folder, Rules \[S8 V\]; Edit Date & Time and Shift Date Range \[S8 V\]
- Presets: New Preset, Apply Preset, Delete Preset, Manage Presets \[S10 V\]
- Copy Settings (All, Modified, Default, None) \[S3 V\]; Select a Lens Profile (Make, Model, Profile) \[S3 V\]; Mask Overlay settings \[S4 V\]
- Enhance dialog with before/after preview \[S14 V\]; Generative Upscale, AI Sharpen, Generate Video dialogs \[S13 S16 V\]
- Inline panels: Generative Expand, Prompt to Edit, Remove tool options \[S12 S15 S17 V\]
- Export screen plus Watermark settings \[S6 V\]; Share & Invite (link, invite, Customise, Settings tabs) \[S6 V\]; Add Connection \[S6 V\]
- People: Are these the same person?, Remove Photo from Person, Show and Hide People, Rename Person \[S19 V\]
- Assisted Culling: Apply Batch Actions, Culling Info \[S18 V\]
- Warning dialogs and tips, resettable in Preferences > Interface \[S5 V\]

### A.3 Verified Windows versus macOS differences

| Item | Windows | macOS | Src |
| --- | --- | --- | --- |
| Preferences entry | Edit > Preferences, Ctrl+, | Adobe Lightroom > Preferences, Cmd+, | S19 S20 |
| Full screen app window | F11 | Shift+Cmd+F | S20 |
| Create Version | Ctrl+Shift+S | Shift+M | S20 |
| Folder import button label | Choose Folder | Review For Import | S9 |
| Reveal local folder | Show in Explorer | Show in Finder | S7 |
| Export context gesture (as published) | right-click | Command-click | S6 C |
| Merge People clusters (multi-select) | Shift-click | Command-click | S19 |
| Change clone/detect source | Ctrl+drag | Cmd+drag | S12 |
| Batch Denoise | Ctrl+Alt+E | Cmd+Option+E (also published as Control+Option+E) | S14 C |
| Minimum OS | Windows 10 22H2 or 11 23H2+ | macOS 14 Sonoma+ | S21 |
| GPU API and floor | DirectX 12, 2 GB VRAM | Metal, 2 GB VRAM | S21 |
| AI headroom | 8 GB dedicated or 16 GB shared | Apple silicon M1+, 16 GB unified | S21 |
| Install restriction | none documented | not on case-sensitive volumes or removable flash | S21 |
| AI Denoise acceleration | GPU | Apple Neural Engine disabled on Apple silicon, macOS 14+ | S14 |

## B. Feature matrix

Availability uses the labels in 0.4. 'same' under Win/mac means no difference was found in the opened pages, not that identity was proven. Src shows source plus status tag.

### B.1 Library, organization and sharing

| Feature | Availability | Entry point | Dependencies and restrictions | Win/mac | Src |
| --- | --- | --- | --- | --- | --- |
| Cloud library views | Available | Left pane > Cloud > All Photos menu | Recently Added, Recent Edits, By Date, By People, Connections, Deleted; needs Cloud sync | same | S7 S8 V |
| Local library (in place) | Available | Left pane > Local | No import; edits live in the file or an XMP sidecar; no whole-folder delete in app; Include subfolders switch | Finder vs Explorer wording | S7 V |
| Photo Grid, Square Grid, Detail | Available | Switcher at bottom of the grid | Edit controls active only in Detail | same | S8 V |
| Filmstrip | Available | Slash key | Shown with Detail view; deeper behavior H | same | S20 V |
| Full-screen viewing | Available | F or View > Detail - Full Screen | Detail only; arrows, Z, X and 0-5 work; F or Esc exits | same | S8 V |
| Albums and folders | Available | Albums panel plus menu | Folders hold albums and subfolders; drag photos onto albums | same | S7 S8 V |
| Smart albums | Available | Create Smart album from filtered results, or Albums plus menu | Title, Folder, Rules; self-updating; counts and thumbnails appear on open | same | S8 V |
| Stacks | Limited (documented as Cloud only) | Right-click > Group Into Stack; Edit > Stacks; Cmd/Ctrl+G | Local behavior H; dedicated page not opened | same | S8 S20 V; page I |
| Ratings and flags | Available | Right-click, Square Grid hover, keys 0-5, Z, U, X | none | same | S8 S20 V |
| Color labels (custom colors since 9.4) | Available | dedicated page not opened | Filterable in the Filter menu | same | S2 S8 V; page I |
| Keywords and People names | Available | K or View > Keywords | People names are added in the Keywords panel | same | S8 S19 V |
| Facet search | Available | Search bar, facet: syntax | 18 facets (C.1.3) | same | S8 V |
| Improved Search (natural language) | Available (9.3) | Search bar | First use indexes the library; Local coverage H | same | S2 S8 V; C |
| Filter and sort | Available | Filter and Sort icons | Options listed in C.1.3 | same | S8 V |
| Capture-time editing | Available | Info panel, pencil beside Captured | Single photo, or Shift Date Range for a group | same | S8 V |
| Batch rename templates | Available (9.x) | page not opened | not asserted | not asserted | S2 I |
| People View | Available, opt-in | Preferences > General > Privacy, then My Photos > People | Cloud photos only; server-side analysis; unavailable for uploads from Illinois and Texas | Merge multi-select modifier differs | S19 V |
| Assisted Culling | Available | Album > Assisted Culling > Cull | Faces panel since 9.4; Local folders can Copy to Cloud | same | S18 V |
| Deleted view | Available | All Photos > Deleted | Retention window and restore flow H | same | S8 V; H |
| Group albums and links | Available | Share & Invite | Invite-only or public link; four roles; contributions deletable for 48 hours | same | S6 V |
| Connections | Available | Sharing panel > Add Connection | Adobe Portfolio, Blurb, WHCC | same | S6 V |

### B.2 Import, storage and sync

| Feature | Availability | Entry point | Dependencies and restrictions | Win/mac | Src |
| --- | --- | --- | --- | --- | --- |
| Add Photos (cloud import) | Available | Header button or File > Add Photos | Review screen, Add To Album; copies files in and uploads originals | Choose Folder vs Review For Import | S9 V |
| Camera or card-reader import | Available | Add Photos menu lists the device; Browse for folders | Review before adding | same | S9 V |
| Duplicate detection | Available | Review screen | 'Previously Added' blocks re-adding; renaming the file is the documented workaround | same | S9 V |
| Open without import | Available | Local tab | Also reads connected drives and camera cards | same | S7 V |
| Copy to Cloud | Available | Folder More menu; Menu > Copy Photo to Album | Cloud and Local copies stay independent; Local edits can refresh the Cloud copy | same | S7 V |
| Archive Cloud photos to disk | Available | Right-click > Archive Photos locally, or Archive Album Locally | Save to; Organize into one folder or by date; Existing Files keep both or replace | same | S7 V |
| Offline albums | Available | Album right-click > Make Album Available Offline | none stated | same | S7 V |
| Cache, smart previews, originals path, NAS | Available | Preferences > Cache | From a Jan 2025 page | same | S5 V |
| Sync state display | Available | Info panel; Filter > Sync Status | Syncing, Synced and Backed Up, Original Stored Locally, Synced From Lightroom Classic | same | S8 V |
| Conflict, failure and quota flows | Unverified | not documented on opened pages | not asserted | not asserted | H |

### B.3 Global editing

| Feature | Availability | Entry point | Dependencies and restrictions | Win/mac | Src |
| --- | --- | --- | --- | --- | --- |
| Histogram, clipping, drag-to-edit | Available | Edit panel; Cmd/Ctrl+0; J | Dragging edits Exposure, Whites, Blacks, Highlights, Shadows (since 9.4) | same | S2 S3 V |
| Profiles | Available | Profile browser | Groups: Adobe Raw, Camera Matching, Adaptive, Legacy, Creative (Artistic, B&W, Modern, Vintage); Amount slider on Adaptive and Creative; XMP import | same | S3 V |
| Light | Available | Edit > Light | Auto; six tone sliders; parametric and Point curves | same | S3 V |
| Color | Available | Edit > Color | Color Mixer, Point Color (with Variance), Color Grading; W picks white balance; WB sliders H | same | S3 S20 V |
| Effects | Available | Edit > Effects | Texture, Clarity, Dehaze, Vignette, Grain | same | S3 V |
| Detail | Available | Edit > Detail | Sharpening, Denoise, Noise Reduction, Color NR; Raw Details and Super Resolution also reachable here | same | S3 S14 V |
| Optics | Available | Edit > Optics | Chromatic aberration, lens corrections, Defringe | same | S3 V |
| Geometry | Available | Edit > Geometry | Upright (Auto, Guided), Transform sliders, Constrain Crop | same | S11 V |
| Crop and rotate | Available | C | Aspect, straighten, overlays, flip; hold Space to pan and zoom while cropping | same | S11 V |
| Lens Blur | Available | dedicated page not opened | AI depth; GPU-heavy per system requirements | same | S21 V; page I |
| Presets | Available | Shift+P | Recommended, Premium, Yours; Amount 0-200; XMP export | same | S10 V |
| Versions | Available | Shift+V | Auto and Named tabs | Create Version key differs | S3 S20 V |
| Copy and paste settings | Available | Photo menu | Selectable subsets; Tools and Geometry excluded by default | same | S3 V |
| Auto Tone | Available | Light > Auto; Shift+A | Moves tone sliders plus Saturation and Vibrance | same | S3 S20 V |
| Reset | Available | Photo > Reset To Original; Reset To Open | Shift+R and Shift+Cmd+R as noted in S3; Windows variant not stated | H | S3 V |
| Show Original | Available | Backslash key | Other compare modes H | same | S20 V |
| HDR and panorama merge | Available | Cmd/Ctrl+H and Cmd/Ctrl+M (Shift for headless) | dedicated page not opened | same | S20 I |
| Edit in Photoshop | Available | Shift+Cmd/Ctrl+E | Other-apps page not opened | same | S20 I |
| Content Credentials | Available | dedicated page not opened | not asserted | not asserted | S1 I |

### B.4 Masking and retouching

| Feature | Availability | Entry point | Dependencies and restrictions | Win/mac | Src |
| --- | --- | --- | --- | --- | --- |
| Select Subject, Sky, Background | Available | Mask tools on the right toolbar | AI selections; subject detection sharpened in 9.4 | same | S2 S4 V |
| Select Landscape | Available | Mask tools | Up to eight elements: Sky, Mountains, Architecture, Vegetation, Water, Snow, Natural Ground, Artificial Ground | same | S4 V |
| Select Objects | Available | Mask tools | Brush or Rectangle Select | same | S4 V |
| Select People | Available | Mask tools | One mask per person with body and face parts | same | S4 V |
| Brush, Linear Gradient, Radial Gradient | Available | B, L, R | Brush has Size, Feather, Flow, Density, Auto Mask; Radial has Feather | same | S4 S20 V |
| Color, Luminance, Depth Range | Available | Mask tools | Depth Range only on photos that carry depth data | same | S4 V |
| Feather and Edge sliders | Available (9.5) | Mask panel | none stated | same | S2 S4 V |
| Mask set operations | Available | Mask panel | Add, Subtract, Intersect, Invert, Duplicate and Invert | same | S4 V |
| Generative Remove | Available | Remove tool (H) with Use generative AI | Three variations; Refresh; credits not deducted per Oct 2025 page | same | S12 V |
| Detect objects | Available | Remove tool | Masks object plus its shadow or reflection; non-generative fill uses Cmd/Ctrl+drag to move the source | Modifier differs | S12 V |
| Clone and Heal | Available | H | Dedicated pages not opened | same | S12 S20 V; pages I |
| Visualize Spots | Available | A while Healing is active | Threshold slider | same | S12 S20 V |
| Remove People, Reflections, Dust | Available | dedicated pages not opened | Reflection Removal improved in 9.6 | same | S2 I |

### B.5 AI, enhancement and generative

| Feature | Availability | Entry point | Dependencies and restrictions | Win/mac | Src |
| --- | --- | --- | --- | --- | --- |
| Denoise | Available | Detail panel, or Photo > Enhance | Raw-centric (see conflict 2); GPU-heavy; writes a new DNG | Apple Neural Engine disabled for AI Denoise on Apple silicon | S14 V; C |
| Raw Details | Available | Photo > Enhance | Bayer and X-Trans raw only; same resolution; new DNG stacked with source | same | S14 V |
| Super Resolution | Available | Photo > Enhance | Raw, JPEG, TIFF; 2x linear; one pass per image | same | S14 V |
| Generative Upscale | Available (9.3) | Right-click or Photo > Enhance with AI | Topaz Gigapixel; 2x or 4x; 10 credits to 25 MP, 20 credits to 56 MP | same | S13 V |
| AI Sharpen | Available (9.4) | Enhance with AI | Default or Strong; optional Topaz denoise; same credit bands | same | S13 V |
| Generative Expand | Available (9.5) | Crop panel > Enable Expand | Firefly Fill & Expand (credits) or Content-Aware Fill (free) | same | S15 V |
| Generate Video | Available (9.4) | Right-click or Photo > Generate Video | Firefly Video, Google Veo 3.1, Veo 3.1 Fast; credits | same | S16 V |
| Prompt to Edit | Limited availability (early access) | Left panel or View > Prompt to Edit | Nano Banana 2 at 2K or 4K; credits; DNG output | same | S17 V; C |
| Edit in Firefly, Firefly boards | Available (9.3) | pages not opened | Cross-product handoff | same | S2 I |
| Technology Preview switches | Unverified | Preferences > Technology Preview | Contents not captured | same | S5 H |

### B.6 Output and platform

| Feature | Availability | Entry point | Dependencies and restrictions | Win/mac | Src |
| --- | --- | --- | --- | --- | --- |
| Export presets | Available | Share icon | JPG (Small), JPG (Large), Original, Previous Settings | same | S6 V |
| Custom export | Available | Shift+E or File > Export | JPG, TIF, DNG, Original; AVIF and JPEG XL conflict (0.6 item 1) | same | S6 V; C |
| Watermark | Available | Export > Include Watermark | Text, or PNG/JPG graphic up to 5 MB; syncs to mobile | same | S6 V |
| Share & Invite | Available | Grid header | Roles, link access, Customise and Settings tabs | same | S6 V |
| Languages | 16 | Preferences > Interface | See S21 | same | S21 V |
| Subscription tiers (free/starter, premium) | Unverified | pages not opened | Page titles imply a free export tier | not asserted | S1 H |

## C. Panel-by-panel UI specification

Each table lists components with type, behavior, states and source. Where Adobe gives no number, none is invented. Hover, focus and disabled states are H unless a row says otherwise.

### C.1 Library, browsing and organization

#### C.1.1 Left pane

| Component | Type | Behavior | Notes and states | Src |
| --- | --- | --- | --- | --- |
| Cloud and Local tabs | Tab pair | Switch library type | Edit-in-place only in Local; Albums, Shared, People, Connections sit under Cloud | S7 V |
| All Photos menu | Dropdown | Recently Added, Recent Edits, By Date, By People, Connections, Deleted | By People needs Enable People View | S8 V |
| Albums panel | List plus add menu | Create Album, Create Smart Album, Create Folder; sort menu | Album sorts: Album Name, Recently Updated, Date Created, Photo Count; display Name & Covers or Name Only | S7 V |
| Album context menu | Context menu | Share To, Share & Invite, Rename Album, Set as Target Album, Make Album Available Offline, Delete Album, Archive Album Locally | Deleting an album keeps its photos in the library | S7 V |
| Target Album | Mode | Pressing T adds the current selection to the target | Used for shared albums | S6 V |
| Shared panel | List | Shared with You, Shared Albums, Shared Photos | Sort icon | S8 V |
| Local Favorites and Browse | Tabs | Favorites show starred folders; Browse shows all folders and drives | Star icon beside a folder bookmarks it | S7 V |
| Folder More menu | Menu | Add as Favorite, Copy to Cloud, Rename Folder (renames on disk), Create Folder, Show in Finder or Explorer | Dragging photos between folders moves files on disk | S7 V |

#### C.1.2 Views, zoom and rotation

| Component | Type | Behavior | Src |
| --- | --- | --- | --- |
| Photo Grid | View | Thumbnails with no borders or badges | S8 V |
| Square Grid | View | Equal squares showing flag, sync status, rating; hovering reveals rating and flag controls | S8 V |
| Detail | View | One photo; the only view where edit controls are active | S8 V |
| Filmstrip | Strip | Toggle with the slash key | S20 V |
| Zoom | Shortcut | Cmd/Ctrl+= zooms in, Cmd/Ctrl+- zooms out; Space toggles zoom states | S20 V |
| Rotate | Shortcut | Cmd/Ctrl+\[ left, Cmd/Ctrl+\] right | S20 V |
| Select All, Select None | Shortcut | Cmd/Ctrl+A, Cmd/Ctrl+D | S20 V |
| Range and additive selection | Interaction | Shift and Cmd/Ctrl modifiers are assumed; not stated on opened pages | H |

#### C.1.3 Search, filter, sort

- **Facets** (type the name plus a colon): keyword, camera, lens, location, rating, flag, flash, orientation, iso, type, f-stop, shutter speed, edited, extension, album, people, focal length, depth map. Free text also matches metadata, keywords, natural-language descriptions, and photos that belong to no album. \[S8 V\]
- **Filter menu**: Ratings (this many stars or higher), Flags (flagged, rejected, unflagged), Color labels, Culling Score, Edited, Type (photos or videos), Keyword, Camera, Location, Sync Status, People. \[S8 V\]
- **Sort menu**: Capture Date, Import Date, Modified Date, File Name, Star Rating. Ascending or descending toggle H. \[S8 V\]
- Any search can be saved as a Smart album. \[S8 V\]

#### C.1.4 Info, Keywords, dates

- **Info panel (I)**: lists metadata and Sync Status; pencil beside Captured opens Edit Date & Time for one photo, or Shift Date Range for many (the newest photo takes the typed value and the rest shift by the same offset). \[S8 V\]
- **Keywords panel (K)**: assign keywords to the selection; People section holds an Add Name field. \[S8 S19 V\]

#### C.1.5 Assisted Culling

| Control | Type | Behavior | Src |
| --- | --- | --- | --- |
| Cull | Button | Starts analysis once an option is chosen | S18 V |
| Subject Sharpness | Toggle plus threshold slider | Selects photos with the subject in focus | S18 V |
| Eyes Sharpness | Toggle plus threshold slider | Selects photos with sharp eyes; sub-option: only photos where eyes are detected | S18 V |
| Eyes Open | Toggle | Selects open-eye photos; sub-options: only detected eyes, include Can't tell images | S18 V |
| Exposure issues, Misfires, Documents and receipts | Toggles (Reject) | Mark matching photos Reject | S18 V |
| Review: Selects, Rejects, All | Segmented control | Filters the grid; Select and Reject badges mark photos | S18 V |
| Apply Batch Actions | Menu (after analysis) | Apply Flag (Pick, Unflagged, Reject), Apply Rating, Apply label, Add to Album, Remove from Album, Delete (permanent) | S18 V |
| Copy to Cloud | Button | Copies selected Local photos to the Cloud | S18 V |
| Culling Info (Info icon) | Panel | Photo section shows Subject Sharpness, Eye Sharpness, Eyes Open scores; Faces section shows per-face scores (since 9.4) | S2 S18 V |
| Select and Reject badges | Badge plus menu | Hover shows criteria and scores; Mark as Select or Mark as Reject overrides in Photo Grid, Square Grid or filmstrip | S18 V |

#### C.1.6 People View

| Control | Behavior | Src |
| --- | --- | --- |
| Enable People View | Preferences > General > Privacy; off by default; disabling deletes face models on the server and stops grouping | S19 V |
| Cluster grid | Cover photo plus count per person; empty state reads 'No People Found Yet' while analysis runs | S19 V |
| Sort | Three-dot menu: First Name, Last Name, Count | S19 V |
| Name, Rename | Add Name field in an open cluster, context menu Add Name or Rename Person, or Keywords panel | S19 V |
| Show and Hide People | Three-dot menu; checklist; Done | S19 V |
| Merge | Multi-select (Cmd-click or Shift-click) then Merge, rename to an existing name, or drag one cluster onto another; suggestion banner offers Yes, Merge or No | S19 V |
| Cover photo | Open cluster, select photo, Set as Cover Photo | S19 V |
| Remove or move photo | Remove Photo from Person (confirm dialog) or Move To another person | S19 V |

### C.2 Import, Local, Cloud and storage

#### C.2.1 Add Photos flow (Cloud)

| Step | Component | Behavior | Notes and states | Src |
| --- | --- | --- | --- | --- |
| 1 | Add Photos | Header button or File > Add Photos; with a camera or card reader attached it opens a menu listing the device plus Browse | Shortcut Shift+Cmd/Ctrl+I | S9 S20 V |
| 2 | Folder or file picker | Pick a folder (button reads Choose Folder on Windows, Review For Import on macOS) or pick files (Review For Import) | none | S9 V |
| 3 | Review screen | Thumbnails; hover reveals a gray circle at top-left, click makes a blue check; Select All at top-right; click a check to deselect | 'Previously Added' overlay marks a duplicate that cannot be added | S9 V |
| 4 | Add To Album | Dropdown: None, New, or an existing album (contributors also see Shared with You albums) | none | S6 S9 V |
| 5 | Add Photos | Copies files in and uploads full-resolution originals; confirmation reads 'Photos will be Synced to Creative Cloud' | Source media can be cleared afterwards | S7 S9 V |
| 6 | Local cache | Imports land in the storage location chosen in Preferences; Cache shows inflated usage until sync completes, then Lightroom prunes it | none | S9 V |

No import-time rename, metadata template or develop preset step is documented on these pages (H). Preferences > Import supplies HDR and RAW defaults (C.10). Duplicate false positives: rename the file outside the app, then add it again. \[S9 V\]

#### C.2.2 Local tab rules

| Rule | Detail | Src |
| --- | --- | --- |
| No import | Photos and videos on disk, connected drives and camera cards open directly | S7 V |
| Where edits live | Non-destructive; stored inside the file for some formats, otherwise in an XMP sidecar next to it | S7 V |
| Moving files | Moves inside Lightroom carry the sidecar; moves outside need the XMP moved by hand | S7 V |
| Visibility to other apps | Edits are invisible elsewhere until you export | S7 V |
| Folder tools | Favorite (star), Rename Folder (renames on disk), Create Folder inside a folder, Show in Finder or Explorer, drag-and-drop between folders (moves files) | S7 V |
| Subfolders | Include subfolders switch shows nested photos in the parent view | S7 V |
| Deletion | Photos and videos can be deleted in-app; whole-folder deletion is not supported (use the OS) | S7 V |
| Copy to Cloud | Folder or photo is copied to the Cloud; copies are independent; re-edit locally and push the new edits to the Cloud copy | S7 V |
| Albums, smart albums, sharing, People | Not documented for Local items | H |

#### C.2.3 Cloud versus Local, side by side

| Capability | Cloud tab | Local tab | Src |
| --- | --- | --- | --- |
| Getting photos in | Add Photos copies and uploads originals | None; open in place | S7 S9 V |
| Originals | Uploaded; local footprint governed by cache and preferences; can be archived to disk | Stay where they are | S5 S7 V |
| Organizing | Albums, folders, smart albums, shared albums, stacks | Folder tree on disk (favorites, rename, subfolders) | S7 S8 V |
| Edit storage | Synced with the cloud library | File metadata or XMP sidecar | S7 V |
| Cross-device access | Mobile and Web via sync | Only after Copy to Cloud | S7 V |
| People, Improved Search | Cloud features (People analysis runs on Adobe servers) | Not documented | S8 S19 V; Local H |
| Stacks | Supported | Documented as Cloud-only for AI outputs | S13 V |
| Offline use | Make Album Available Offline | Always local | S7 V |
| Culling | Assisted Culling in albums | Works in Local folders, plus Copy to Cloud | S18 V |
| Delete and recovery | Deleted view (restore flow H) | In-app delete of photos; no folder delete | S7 S8 V |

#### C.2.4 Storage and sync controls

- **Archive Cloud to disk**: right-click photos > Archive Photos locally, or an album > Archive Album Locally; dialog fields Save to (Browse), Organize (Into one folder or By date), Existing Files (Keep both or Replace); album structure can be preserved. \[S7 V\]
- **Preferences > Cache** (Jan 2025 page): app and user data size, stored-offline size, photo cache, free space, cache size limit, Store a Copy of All Smart Previews Locally, Storage Location for Originals (Browse or Reset), and support for network-attached storage. \[S5 V\]
- **Preferences > Account**: Manage Account and Cloud Storage. \[S5 V\]
- **Preferences > General**: prevent system sleep while syncing on power. \[S5 V\]
- **Sync status vocabulary**: Syncing, Synced and Backed Up, Original Stored Locally, Synced From Lightroom Classic (filter and Info panel). \[S8 V\]
- **Not documented on opened pages (H)**: sync conflicts, interrupted transfers, quota-exceeded behavior, missing-file relinking, smart preview regeneration, upload pause or resume controls.

### C.3 Edit panel sections

Numeric ranges and defaults are not stated on the opened pages except where shown; every other range is H. Slider mechanics (drag, numeric entry, reset on double-click) are assumed conventions (H) unless a row cites a source.

#### C.3.0 Shell

| Component | Behavior | Src |
| --- | --- | --- |
| Edit panel icon (upper-right) | Opens the Edit panel; controls are live only in Detail view | S3 S8 V |
| Section shortcuts | Cmd/Ctrl+1 Light, 2 Color, 3 Effects, 4 Detail, 5 Optics, 6 Geometry; Cmd/Ctrl+0 Histogram | S20 V |
| Panel layout | Preferences > Interface: Edit Panel Layout; Panel Tracks Automatic or Manual | S5 V |
| Hover tips | Animated hover tips and feature intros can be switched off in Preferences > Interface | S5 V |

#### C.3.1 Histogram

| Control | Behavior | Src |
| --- | --- | --- |
| Show Histogram | Three-dot menu entry; Cmd/Ctrl+0 | S3 S20 V |
| Clipping overlay | Blue marks clipped shadows, red marks clipped highlights; J toggles; right-click (Cmd-click) the histogram > Show Clipping in Histogram | S3 S20 V |
| Drag-to-edit tone | Dragging across histogram regions changes Exposure, Whites, Blacks, Highlights, Shadows (since 9.4) | S2 S3 V |
| Channel view, RGB readout on hover | Not documented | H |

#### C.3.2 Profile browser

| Control | Behavior | Src |
| --- | --- | --- |
| Browse | Opens the browser; profiles preview on the photo | S3 V |
| Groups | Adobe Raw, Camera Matching, Adaptive, Legacy (raw), Creative: Artistic, B&W, Modern, Vintage | S3 V |
| Amount slider | Present for Adaptive, Artistic, B&W, Modern, Vintage | S3 V |
| Favorites | Star icon pins profiles | S3 V |
| Layouts | List, Grid, Large thumbnails | S3 V |
| Filter | Color or B&W | S3 V |
| Import and manage | Three-dot menu: Import Profiles (XMP), Manage Profiles, show or hide groups per device | S3 V |
| Sync | Profiles and presets sync across desktop and mobile but not to Classic | S3 V |
| Defaults | Adobe Color for color and Adobe Monochrome for B&W | S3 V |
| Interaction with existing edits | Not documented | H |

#### C.3.3 Light

| Control | Type | Behavior | Src |
| --- | --- | --- | --- |
| Auto | Button | Sets Exposure, Contrast, Highlights, Shadows, Whites, Blacks, Saturation, Vibrance; Shift+A | S3 S20 V |
| Exposure, Contrast, Highlights, Shadows, Whites, Blacks | Sliders | Range and default not stated | S3 V; ranges H |
| Curve: parametric | Draggable curve segments | Drag regions of the curve | S3 V |
| Curve: Point Curve | Graph | Channels RGB, Red, Green, Blue; add and delete control points; Reset Channel | S3 V |
| Curve presets | Dropdown | Linear, Medium Contrast, Strong Contrast | S3 V |
| Refine Saturation | Slider | Tames saturation shifts caused by curve changes | S3 V |

#### C.3.4 Color

| Control | Type | Behavior | Src |
| --- | --- | --- | --- |
| White Balance, Temperature, Tint, Vibrance, Saturation | Dropdown, sliders | Not captured on the opened page; W activates the white-balance sampler | S20 V; rest H |
| Color Mixer | Hue, Saturation, Luminance per color | Targeted adjustment: drag directly on the photo; floating toolbar | S3 V |
| Point Color | Picker plus swatches | Up to 8 swatches; Hue Shift, Saturation Shift, Luminance Shift, Variance; Hue, Saturation, Luminance Range; Visualize Range; reset by button or double-click | S2 S3 V |
| Color Grading | Four wheels | Midtones, Shadows, Highlights, Global; each with Hue, Saturation, Luminance; plus Blending and Balance | S3 V |
| B&W conversion | Profile groups | B&W creative profiles and Adobe Monochrome; per-channel B&W mixer not documented | S3 V; mixer H |

#### C.3.5 Effects

| Control | Notes | Src |
| --- | --- | --- |
| Texture, Clarity, Dehaze | Sliders | S3 V |
| Vignette | Midpoint, Roundness, Feather, Highlights (plus an amount slider, H) | S3 V |
| Grain | Size and Roughness (plus an amount slider, H); S3 also appears to list Grain under Detail | S3 V; C |
| Lens Blur | Dedicated page not opened; AI depth map; GPU-heavy | S21 V; page I |

#### C.3.6 Detail

| Control | Notes | Src |
| --- | --- | --- |
| Sharpening | Amount, Radius, Detail, Masking | S3 V |
| Denoise (AI) | Opens the Enhance dialog: Noise Reduction amount slider, press-and-hold preview, Enhance button; writes a new DNG; batch via Cmd+Option+E or Ctrl+Alt+E on selected photos | S14 V |
| Noise Reduction | Detail and Contrast sliders (primary amount slider H) | S3 V |
| Color Noise Reduction | Amount, Detail, Smoothness | S3 V |
| Raw Details, Super Resolution | Enhance dialog options; file types and limits in C.8 | S14 V |

#### C.3.7 Optics

| Control | Notes | Src |
| --- | --- | --- |
| Remove Chromatic Aberration | Checkbox | S3 V |
| Enable Lens Corrections | Checkbox; a message states when a built-in profile is used | S3 V |
| Select a Lens Profile | Dialog with Make, Model, Profile | S3 V |
| Distortion Correction | Slider, default 100 | S3 V |
| Lens Vignetting | Slider, default 100 | S3 V |
| Defringe | Purple or Green choice, fringe selector, Amount and Hue sliders | S3 V |
| Camera and lens coverage | Lists live on Adobe's supported-cameras and supported-lenses pages (linked from What's New) | S2 I |

#### C.3.8 Crop, rotate and Geometry

| Control | Behavior | Src |
| --- | --- | --- |
| Crop overlay handles | Drag to resize; hold Space to pan and zoom while cropping | S11 V |
| Aspect menu | Picks the ratio; list of presets and custom entry not documented | S11 V; list H |
| Rotate Aspect Ratio | Flips the crop 90 degrees | S11 V |
| Constrain Aspect Ratio lock | Unlock to crop freely | S11 V |
| Straighten slider | Whole-image alignment | S11 V |
| Crop Overlay dropdown | Composition guides; O cycles overlays | S11 S20 V |
| Rotate and flip | 90-degree left and right, vertical flip, horizontal mirror | S11 V |
| Upright menu | Auto Upright icon; Guided Upright (draw guides on the photo; Shift+G) | S11 S20 V |
| Transform sliders | Distortion, Vertical, Horizontal, Rotate, Aspect, Scale, X Offset, Y Offset | S11 V |
| Constrain Crop checkbox | Keeps the crop inside visible pixels | S11 V |
| Enable Expand | Turns the crop into a Generative Expand session (C.8) | S15 V |
| Reset and confirm | Not documented | H |

### C.4 Masking and local adjustments

Entry: open a photo in Detail view and use the mask tools on the right-hand toolbar; exact rail icons are H. \[S4 V\]

#### C.4.1 Mask creation tools

| Tool | Key | Behavior and parameters | Src |
| --- | --- | --- | --- |
| Select Subject | - | AI finds the main subject; precision improved in 9.4 | S2 S4 V |
| Select Sky | - | AI sky mask | S4 V |
| Select Background | - | AI background mask | S4 V |
| Select Landscape | - | Up to eight element masks: Sky, Mountains, Architecture, Vegetation, Water, Snow, Natural Ground, Artificial Ground | S4 V |
| Select Objects | - | Define the area with Brush or Rectangle Select; AI snaps to the object | S4 V |
| Select People | - | Person 1, 2 and so on; choose parts such as skin, beard, clothes, hair, teeth | S4 V |
| Brush | B | Size, Feather, Flow, Density, Auto Mask; bracket keys change size and Shift plus bracket changes feather (direction conflict, 0.6 item 4) | S4 S20 V; C |
| Linear Gradient | L | Drag to draw; handle behavior H | S4 S20 V |
| Radial Gradient | R | Drag to draw an ellipse; Feather slider | S4 S20 V |
| Color Range | - | Click to sample up to five colors (Shift adds, Alt or Option removes); Refine slider | S4 V |
| Luminance Range | - | Select Luminance slider; Show Luminance Map | S4 V |
| Depth Range | - | Only on photos with depth data; Select Depth slider; Show Depth Map | S4 V |

#### C.4.2 Mask management

| Control | Behavior | Src |
| --- | --- | --- |
| Masks panel | Floating panel listing masks; Create New Mask; preset dropdown (Lighten, Darken, Warmer, Cooler and others) | S4 V |
| Naming | Default Mask 1, Mask 2; rename from three-dot menu or right-click | S4 V |
| Add and Subtract | Any tool can add to or subtract from the active mask | S4 V |
| Intersect | 'Intersect with Mask using' another tool | S4 V |
| Invert | Invert Mask; Duplicate and Invert | S4 V |
| Delete | Not captured in the notes | H |
| Overlay | Default red; Mask Overlay settings: color picker, opacity, unaffected-areas mode (Color Overlay on B&W, Image on B&W, Image on Black, Image on White); O cycles overlay, Shift+O cycles color | S4 S20 V |
| Copy and batch | Copy Edit Settings; Paste to Entire Selection from the three-dot menu | S4 V |
| AI processing and failure states | Not documented on the opened page | H |

#### C.4.3 Per-mask adjustment set

Feather and Edge (since 9.5), Exposure, Contrast, Highlights, Shadows, Whites, Blacks, Curves, Temp, Tint, Hue, Saturation, Colorize, Texture, Clarity, Dehaze, Grain (Size, Roughness), Sharpness, Noise Reduction, Moire, Defringe, Refine Saturation. \[S2 S4 V\]

Compared with the global Edit panel, the local set adds Feather, Edge, Moire and Colorize and omits Profile, Vibrance, Color Mixer, Point Color, Color Grading, Optics and Geometry (based on the two lists; absence of a control is H).

### C.5 Remove and retouch

#### C.5.1 Remove tool (Generative Remove)

| Control | Behavior | Src |
| --- | --- | --- |
| Entry | Remove tool in Detail view; H is the published key for the healing brush | S12 S20 V |
| Use generative AI | Toggle; Firefly-powered fill. With it on, Generate produces three variations and Refresh requests three more | S12 V |
| Detect objects | Toggle; circle or scribble over an object and the tool masks it, including a shadow or reflection it casts | S12 V |
| Both toggles on | Detection first, then three generative variations | S12 V |
| Detect objects only | Non-generative fill from a source region; hold Cmd/Ctrl and drag to move the source | S12 V |
| Add, Subtract | Grow or shrink the brushed area | S12 V |
| Size | Brush size slider | S12 V |
| Overlay color, Show Overlay on Hover | Visibility aids | S12 V |
| Visualize Spots | Highlights blemishes; Threshold slider; A toggles it | S12 S20 V |
| Credits | Page states generative credits are not deducted (Oct 2025) | S12 V |
| Copy and paste | Generative removals travel with copied edit settings | S12 V |
| Feedback and guidelines | Feedback button; link to Adobe's generative AI user guidelines | S12 V |
| Commit, revise, discard | Variation picker confirmation and cancel behavior not captured | H |

#### C.5.2 Other retouching tools (index-only)

Clone, Heal, Remove People, Remove Reflections and Remove Dust each have a dedicated Adobe page that was not opened. Known facts: People Removal and Reflection Removal arrived together in mid 2025 under distraction removal, Reflection Removal was improved in 9.6, dust-spot removal arrived with the October 2025 wave, and Reflection Removal is one of the GPU-hungry AI features named in the system requirements. \[S2 S21 I\] Controls, overlays, sampling and result selection for these tools are H.

### C.6 Presets

| Control | Behavior | Src |
| --- | --- | --- |
| Presets panel | Shift+P; groups Recommended, Premium, Yours | S10 V |
| Recommended | AI-recommended presets with category chips such as Subtle, Strong, B&W, Cool, Warm, Dark, Bright, Cinematic, HDR | S10 V |
| Premium | Adaptive, Portraits, Auto+, Seasons, Style, Subject, Video; Film-Inspired group added in 9.3 | S2 S10 V |
| Yours | User-created presets | S10 V |
| Apply and browse | Click to apply; arrow keys step through presets; hover preview H | S10 V |
| Amount | Slider from 0 to 200 | S10 V |
| Favorites | Star pins a preset | S10 V |
| Layout | List view, Single-Group Mode, Collapse All Groups | S10 V |
| Compatibility | Partially compatible presets are still listed; More like this suggestions | S10 V |
| Create Preset | New Preset dialog documented as name-only | S10 V; C |
| Update With Current Settings | Overwrites a user preset with the photo's current settings | S10 V |
| Batch apply | Apply Preset dialog on a multi-selection | S10 V |
| Export preset | Writes an XMP file | S10 V |
| Delete, Manage Presets | Menu items | S10 V |
| Import, third-party presets, mobile sync | Separate Adobe page (How to Add Presets / Sync Presets with mobile) not opened | S1 I |
| Reset and interaction with existing edits | Not documented | H |

### C.7 Editing utilities

| Utility | Behavior | Src |
| --- | --- | --- |
| Copy Edit Settings | Photo menu; Cmd/Ctrl+C | S3 S20 V |
| Choose Edit Settings to Copy | Shift+Cmd/Ctrl+C; Copy Settings dialog with Select All, Modified, Default, None; Tools and Geometry unchecked by default | S3 V |
| Paste Edit Settings | Cmd/Ctrl+V; Paste Selected Settings is published as Shift+Ctrl+V on both platforms | S3 S20 V; C |
| Paste to entire selection | Mask-level command on the three-dot menu | S4 V |
| Auto Tone | Shift+A or the Auto button in Light | S3 S20 V |
| Reset To Original, Reset To Open | Photo menu; Shift+R and Shift+Cmd+R as noted from S3 | S3 V |
| Undo, Redo | Cmd/Ctrl+Z, Shift+Cmd/Ctrl+Z; which actions are undoable is H | S20 V |
| Show Original | Backslash key | S20 V |
| Before and after layouts | Split or side-by-side compare not documented | H |
| Versions | Versions icon in the sidebar (since 6.4, June 2023); Shift+V expands it; Auto tab collects automatic versions including collaborators' edits; Named tab holds saved versions | S3 S6 S20 V |
| Create or save a named version | Create Version (Shift+M on macOS, Ctrl+Shift+S on Windows) or Save as Named Version | S3 S20 V |
| Version rows | Hover previews the version; three-dot menu renames or deletes; Export with these settings exports that version's edits | S3 V |
| Batch and sync | Apply Preset on multi-selection and Paste to Entire Selection exist; Classic-style Sync Settings is not documented | S4 S10 V; sync H |
| History panel, snapshots | Not documented; do not import Classic's model | H |

### C.8 AI and generative feature register

Status labels follow 0.4. No feature in the opened pages is marked Technology Preview or Announced-only; Preferences carries a Technology Preview tab whose contents were not captured (H). Credit amounts come from the pages cited and may change.

| Feature | Status | Entry | Controls and workflow | Output, credits, dependencies | Src |
| --- | --- | --- | --- | --- | --- |
| Denoise (AI) | Available | Detail panel, or Photo > Enhance; batch Cmd+Option+E or Ctrl+Alt+E | Noise Reduction amount slider, press-and-hold preview, Enhance | New DNG (Enhance-NR suffix); GPU-heavy; raw-centric; Apple Neural Engine off on Apple silicon | S14 V; C |
| Raw Details | Available | Photo > Enhance, or context menu Enhance | Enhance dialog, drag-to-inspect preview | New DNG stacked with source; Bayer and X-Trans raw only | S14 V |
| Super Resolution | Available | Photo > Enhance | Same dialog; preview at 2x | 2x linear (4x pixels); raw, JPEG, TIFF; one pass per image | S14 V |
| Generative Upscale | Available (9.3) | Right-click > Enhance with AI, or Photo > Enhance with AI | Scale 2x or 4x; size preview; credit balance; Create stack; Upscale | New image beside or stacked; Topaz Gigapixel; 10 credits to 25 MP, 20 credits to 56 MP; progress shown in header | S13 V |
| AI Sharpen | Available (9.4) | Same menu | Strength Default or Strong; Apply Topaz Denoise; Create stack; Sharpen | New image; same credit bands | S13 V |
| Generative Expand | Available (9.5) | Crop panel > Enable Expand | Method Firefly Fill & Expand (three variations) or Content-Aware Fill (one); Generate, Generate More, variation arrows, hold eye icon to compare, Remove all generations, Keep, Cancel | Firefly path uses credits, Content-Aware Fill is free; expand one edge at a time; works after perspective correction and on panoramas | S15 V |
| Generate Video | Available (9.4) | Right-click or Photo > Generate Video | Model (Firefly Video, Google Veo 3.1, Veo 3.1 Fast), aspect ratio, resolution, duration, prompt with presets Slow pan, Subtle motion, Hyperlapse; Create stack | New video beside or stacked; credits including partner-model credits | S16 V |
| Prompt to Edit | Limited availability (early access) | Left panel, or View > Prompt to Edit | Prompt field with example prompts (for instance dust and scratches, color restoration, unblur, sky replacement); Recent and saved prompts with save and delete; model and size Nano Banana 2 at 4K or 2K; Create stack; Generate | New DNG beside or stacked; credit balance icon; stacks Cloud-only | S17 V |
| Generative Remove | Available | Remove tool | See C.5.1 | Credits not deducted per Oct 2025 page | S12 V |
| AI mask selections | Available | Mask tools | Subject, Sky, Background, Landscape, Objects, People, Depth | GPU; see C.4 | S4 V |
| Assisted Culling | Available | Album > Assisted Culling | See C.1.5 | Faces panel since 9.4 | S18 V |
| Improved Search | Available (9.3) | Search bar | Natural-language queries | Indexing on first use | S8 V; C |
| People View | Available, opt-in | Preferences > General | See C.1.6 | Server-side; region exclusions | S19 V |
| Lens Blur, Reflection Removal | Available | Pages not opened | AI depth and AI reflection removal | Named GPU-hungry features | S21 V; pages I |
| Adaptive profiles and presets | Available | Profile browser, Presets panel | Amount slider | Best on HDR raw per S2 | S2 S3 V |
| Edit in Firefly, Firefly boards | Available (9.3) | Pages not opened | Cross-product handoff | Not asserted | S2 I |

### C.9 Export and sharing

#### C.9.1 Export controls

| Control | Values and behavior | Src |
| --- | --- | --- |
| Share menu presets | JPG (Small), JPG (Large), Original, Previous Settings, Custom Settings; each then asks for a destination | S6 V |
| Custom Settings entry | Shift+E, File > Export, or context menu Export; Cmd/Ctrl+E repeats the previous export including its location | S6 V |
| Preset dropdown | Upper-right of the Export screen; fills the panel from a preset | S6 V |
| File Type | JPG, TIF, DNG, Original (AVIF and JPEG XL appear only in the page intro) | S6 V; C |
| Dimensions | Small (long edge 2048 px), Full Size, Custom (long and short edge in pixels, max 65,000) | S6 V |
| Quality | 10 percent to 100 percent | S6 V |
| Include Watermark | Checkbox with a settings icon (Text or Graphic) | S6 V |
| More Options: metadata | All metadata, or copyright only | S6 V |
| More Options: File Naming | Original name, custom name, or date-filename | S6 V |
| More Options: Output Sharpening | Medium and amount | S6 V |
| More Options: Color Space | Chosen by intended output with in-app tooltips; the list of spaces is not documented | S6 V; list H |
| Export Photo button | Shows how many photos will export; then an OS destination picker (local, external or network drive) | S6 V |
| Exporting a version | Versions row three-dot menu: Export with these settings | S3 V |
| Overwrite handling, resolution (ppi), progress, failure and completion states, video export | Not documented on the page | H |

#### C.9.2 Watermark

| Tab | Controls | Src |
| --- | --- | --- |
| Text | Text field (defaults to your name), Font, Style (Regular, Bold, Italic, Bold Italic), Rotate, Color (black or white), Use Drop Shadow, Anchor grid, Size, Opacity, Horizontal and Vertical Offset; live preview on the photo | S6 V |
| Graphic | Upload PNG or JPG (transparent background advised, 5 MB cap), Rotate, Anchor, Size, Opacity, Offsets | S6 V |
| Sync | Settings sync to Lightroom mobile; unsupported fonts fall back there | S6 V |

#### C.9.3 Share & Invite (group albums)

| Element | Behavior | Src |
| --- | --- | --- |
| Entry | Context menu on an album or selection, header icon, or three-dot menu; shared albums also appear in Shared to Web | S6 V |
| Get shareable link | Link Access defaults to Invite only; switch to Anyone can view for a public link; copy button; Facebook and Twitter icons for public links | S6 V |
| Invite | Enter emails; per-person role Can View, Can Contribute, Can Edit and Contribute (edits appear in the Auto tab of Versions; invitees can export JPG), Remove Access; approve or deny access requests | S6 V |
| Customise tab | Title toggle, Author toggle, Theme (Photo Grid, One-Up, Column), Appearance (Dark or Light), Customise on the web | S6 V |
| Settings tab | Allow access requests, Anyone can upload, Allow JPG downloads, Show metadata, Show location data, Allow comments and likes | S6 V |
| Invitee side | Email View album button opens Lightroom on the web; desktop shows the Shared tab; contribute by drag-and-drop, Target Album plus T, or Add Photos with Shared with You album | S6 V |
| Limits | Contributions can be deleted for 48 hours; contributed photos exportable only with download access | S6 V |

#### C.9.4 Connections

Sharing icon in the left pane > Connections > Add Connection (Adobe Portfolio, Blurb, WHCC). Share To from a photo or album context menu names a project; photos can be added to an existing project or removed from the public set without deleting them; Remove Connection leaves published content online. \[S6 V\]

### C.10 Menus, context menus and preferences

#### C.10.1 Context and overflow menus

| Surface | Items | Src |
| --- | --- | --- |
| Photo (grid or filmstrip) | Set Rating, Set Flag, Group Into Stack, Ungroup Stack, Share & Invite, Share To, Export, Archive Photos locally (Cloud), Copy Photo to Album (Local), Enhance with AI, Generate Video, Enhance | S6 S7 S8 S13 S14 S16 V |
| Album | Share To, Share & Invite, Rename Album, Set as Target Album, Make Album Available Offline, Delete Album, Archive Album Locally | S7 V |
| Smart album | Edit, Rename, Delete | S8 V |
| Local folder (More menu) | Add as Favorite, Copy to Cloud, Rename Folder, Create Folder, Show in Finder or Explorer | S7 V |
| People cluster | Add Name, Rename Person; inside a cluster: Set as Cover Photo, Remove Photo from Person, Move To | S19 V |
| People view overflow | Sort By (First Name, Last Name, Count), Show and Hide People, Rename Person | S19 V |
| Connection | Remove Connection | S6 V |
| Histogram | Show Histogram, Show Clipping in Histogram | S3 V |
| Profile browser overflow | Import Profiles, Manage Profiles, group visibility | S3 V |
| Presets overflow | Create Preset, Update With Current Settings, Export, Delete, Manage Presets, list and group options | S10 V |
| Mask overflow | Rename, Invert, Duplicate and Invert, Intersect, Paste to Entire Selection | S4 V |
| Version row overflow | Rename, Delete, Export with these settings | S3 V |
| Grid header overflow | Share & Invite | S6 V |
| Toolbar overflow and Window, Help menus | Not documented | H |

#### C.10.2 Preferences dialog (Jan 2025 page; may lag v9.6)

Open with Edit > Preferences (Windows) or Adobe Lightroom > Preferences (macOS), or Ctrl/Cmd+comma; a Done button closes it. \[S5 S19 S20 V\]

| Tab | Controls | Src |
| --- | --- | --- |
| Account | Manage Account; Cloud Storage | S5 V |
| Cache | App and user data size; stored offline; photo cache; free space; cache size limit; Store a Copy of All Smart Previews Locally; Storage Location for Originals (Browse, Reset); network-attached storage | S5 V |
| General | Prevent system sleep during sync while on power; Privacy: Enable People View | S5 S19 V |
| Import | HDR defaults; RAW defaults chosen from Adobe Default, Camera Settings, or Presets | S5 V |
| Interface | Language; Text Size; Edit Panel Layout; Panel Tracks (Automatic or Manual); swipe between images; Help: Enable Animated Hover Tips, Enable Feature Tutorials and Intros; Prompts: Reset All Warning Dialogs and Tips | S5 V |
| Performance | GPU mode Auto, Off or Custom; Custom splits Use GPU for Display, Image Processing and Export | S5 V |
| Technology Preview | Experimental switches; contents not captured | S5 H |

Languages offered: Deutsch, Dutch, English, Espanol, Francais, Italiano, Norwegian, Portugues (Brasil), Polish, Russian, Svenska, Thai, Japanese, Simplified Chinese, Traditional Chinese, Korean. Minimum display is 1280 x 768 and 1920 x 1080 or more is recommended. \[S21 V\]

### C.11 Keyboard shortcuts (Desktop page, Jun 2025)

Source for every row is S20 unless noted; rows marked C are internally inconsistent on Adobe's page. 'Detail' means the shortcut acts in Detail view.

#### Views and panels

| Action | Windows | macOS | Context |
| --- | --- | --- | --- |
| Crop, Rotate, Geometry | C | C | Detail |
| Detail view | D | D | Global |
| Edit panel | E | E | Detail |
| Grid view | G | G | Global |
| Info panel | I | I | Global |
| Keywords panel | K | K | Global |
| Photos browser | P | P | Global |
| Presets panel | Shift+P | Shift+P | Detail |
| Versions panel (expand) | Shift+V | Shift+V | Detail |
| Full screen for the photo | F (Esc exits) | F (Esc exits) | Detail |
| App full screen | F11 | Shift+Cmd+F | Global |
| Toggle filmstrip | / | / | Detail |
| Show or hide histogram | Ctrl+0 | Cmd+0 | Edit |
| Light, Color, Effects, Detail, Optics, Geometry | Ctrl+1 to Ctrl+6 | Cmd+1 to Cmd+6 | Edit |
| Zoom in, zoom out | Ctrl+= , Ctrl+- | Cmd+= , Cmd+- | Detail |
| Toggle zoom states | Space | Space | Detail |
| Rotate left, right | Ctrl+\[ , Ctrl+\] | Cmd+\[ , Cmd+\] | Detail (label says CCW for right; C) |
| Show Original | backslash key | backslash key | Detail |

#### Library and organization

| Action | Windows | macOS | Context |
| --- | --- | --- | --- |
| Add Photos | Ctrl+Shift+I | Shift+Cmd+I | Global |
| Select all, select none | Ctrl+A , Ctrl+D | Cmd+A , Cmd+D | Grid |
| Search | Ctrl+F | Cmd+F | Global |
| Pick, remove pick, reject | Z , U , X | Z , U , X | Grid and Detail |
| Star rating | 0 to 5 | 0 to 5 | Grid and Detail |
| Group into stack, ungroup | Ctrl+G , Ctrl+Shift+G | Cmd+G , Shift+Cmd+G | Grid |
| Remove photo | Delete | Delete | Grid |
| Add selection to target album (S6) | T | T | Grid |

#### Editing, masks and retouching

| Action | Windows | macOS | Context |
| --- | --- | --- | --- |
| Auto Tone | Shift+A | Shift+A | Detail |
| Undo, redo | Ctrl+Z , Ctrl+Shift+Z | Cmd+Z , Shift+Cmd+Z | Global |
| Copy, paste edit settings | Ctrl+C , Ctrl+V | Cmd+C , Cmd+V | Detail |
| Choose edit settings to copy (S3) | Ctrl+Shift+C | Shift+Cmd+C | Detail |
| Paste selected settings | Shift+Ctrl+V as published | Shift+Ctrl+V as published (C) | Detail |
| Reset To Original (S3) | Shift+R | Shift+R | Detail |
| Reset To Open (S3) | not stated | Shift+Cmd+R | Detail |
| Create Version | Ctrl+Shift+S | Shift+M | Detail |
| Toggle clipping overlays | J | J | Detail |
| White balance sampler | W | W | Detail |
| Guided Upright | Shift+G | Shift+G | Geometry |
| Edit in Photoshop | Ctrl+Shift+E | Shift+Cmd+E | Detail |
| Brush mask | B | B | Detail |
| Linear, Radial Gradient | L , R | L , R | Detail |
| Healing brush (Remove) | H | H | Detail |
| Visualize Spots | A (Healing active) | A (Healing active) | Detail |
| Brush size | \[ larger, \] smaller as published (C) | same (C) | Brush |
| Brush feather | Shift+\[ and Shift+\] (direction H) | same | Brush |
| Cycle mask overlay, mask color | O , Shift+O | O , Shift+O | Masks |

#### Export, merge, enhance and app

| Action | Windows | macOS | Context |
| --- | --- | --- | --- |
| Export with Custom Settings (S6) | Shift+E | Shift+E | Global |
| Export with previous settings (S6) | Ctrl+E | Cmd+E | Global |
| Enhance dialog (S14) | Ctrl+Alt+E | Control+Option+E, or Cmd+Option+E in the batch tip (C) | Detail or grid |
| Merge to HDR, headless | Ctrl+H , Ctrl+Shift+H | Cmd+H , Shift+Cmd+H (C: Cmd+H also hides the app) | Grid |
| Merge to Pano, headless | Ctrl+M , Ctrl+Shift+M | Cmd+M , Shift+Cmd+M (C: Cmd+M also minimizes) | Grid |
| Preferences | Ctrl+, | Cmd+, | Global |
| Close window | Ctrl+W | Cmd+W | Global |
| Minimize | Ctrl+M (C) | Cmd+M (C) | Global |
| Hide Lightroom, hide others | Ctrl+H (C), Alt+Ctrl+H | Cmd+H , Option+Cmd+H | Global |
| Quit | Ctrl+Q | Cmd+Q | Global |

No shortcuts are published for Prompt to Edit, Generative Expand, Generate Video, Upscale or AI Sharpen (H).

### C.12 Interaction specification

| Interaction | Documented behavior | Status |
| --- | --- | --- |
| Click and double-click | Double-click a thumbnail opens it; double-click resets Point Color controls; other slider resets H | V for the two, H otherwise \[S19 S3\] |
| Drag and drop | Photos onto albums; albums into folders; photos between Local folders (moves files); clusters onto clusters (merge prompt); photos onto shared albums | V \[S6 S7 S8 S19\] |
| Mouse wheel and trackpad | Swipe between images exists as a preference; wheel zoom and gestures not documented | V and H \[S5\] |
| Zoom and pan | Space toggles zoom states; Ctrl/Cmd plus or minus zooms; Space pans and zooms while cropping | V \[S11 S20\] |
| Keyboard navigation | Arrow keys step through photos in full screen and through presets | V \[S8 S10\] |
| Modifier behavior | Shift or Cmd click multi-selects People clusters; Cmd/Ctrl-drag moves the clone source; Shift adds and Alt or Option removes Color Range samples; Shift+bracket for feather | V \[S19 S12 S4 S20\] |
| Range and additive photo selection | Not documented | H |
| Slider entry | Dragging and numeric entry assumed | H |
| Context menus | Inventory in C.10.1 | V \[various\] |
| Tool activation | B, L, R, H, C keys; Esc leaves full screen; tool cancel and confirm semantics other than Keep or Cancel in Generative Expand | V and H \[S15 S20\] |
| Undo and redo coverage | Not documented | H |
| Hover | Animated hover tips (switchable); Square Grid shows rating and flag on hover; Select and Reject badges show criteria; Versions rows preview; Remove tool can show overlay on hover | V \[S5 S8 S18 S3 S12\] |
| Focus and accessibility | Text Size preference exists; focus order, screen-reader support not documented | H |
| Modal and unsaved state | Warning dialogs and tips can be reset; per-dialog behavior not documented | H \[S5\] |
| Background work | AI jobs show progress at the upper-left of the header and drop results into the filmstrip; People analysis runs on Adobe servers even if the app is closed; cancel behavior H | V and H \[S13 S19\] |

## D. Workflow specification

Workflows D1 to D4 and D6 to D12 restate Adobe's documented steps. D5 is a synthesized sequence that follows the verified panel order; Adobe does not prescribe an editing order.

### D1. Import photos into the Cloud library \[S7 S9\]

1. Connect a camera or card reader if needed and open Lightroom.
2. Click Add Photos (or press Shift+Cmd/Ctrl+I) and pick the device, or Browse.
3. Choose a folder (Choose Folder on Windows, Review For Import on macOS) or select files and click Review For Import.
4. In the review screen click the hover circle on each wanted thumbnail, or tick Select All; skip photos marked Previously Added.
5. Choose Add To Album: None, New, or an existing album.
6. Click Add Photos and accept the 'Photos will be Synced to Creative Cloud' prompt.
7. Watch Sync Status in the Info panel or the Filter menu; once Synced and Backed Up the source media may be cleared.

### D2. Edit files in place with the Local tab \[S7\]

1. Photos > Local > Browse; open a folder and optionally star it for Favorites.
2. Turn on Include subfolders if the shoot is nested.
3. Open a photo in Detail view and edit; changes are stored in the file or an XMP sidecar.
4. Export when other apps need to see the result.
5. To reach the photo on mobile or web, Copy to Cloud; re-copy after later local edits to refresh the Cloud copy.

### D3. Organize a library \[S8 S19\]

1. Rate (0 to 5), flag (Z, U, X) and add keywords (K).
2. Create an album with the plus menu, drag photos in, or set a Target Album and press T.
3. Group related frames into stacks (Cloud).
4. Find photos with facets, filters and natural-language search; save a good search as a Smart album.
5. Optionally enable People View, name clusters and merge duplicates.
6. Correct capture times from the Info panel.

### D4. Cull a shoot with Assisted Culling \[S18\]

1. Open an album and choose Assisted Culling > Cull.
2. Tick Select criteria (Subject Sharpness, Eyes Sharpness, Eyes Open) and Reject criteria (Exposure issues, Misfires, Documents and receipts); analysis starts when an option is ticked.
3. Review Selects, Rejects or All; hover badges for criteria and open Culling Info for scores; override with Mark as Select or Mark as Reject.
4. When analysis finishes use Apply Batch Actions (flag, rating, label, add or remove album, delete).
5. For Local folders, Copy to Cloud the keepers.

### D5. Edit a raw photo end to end (synthesized) \[S3 S4 S11 S12 S14\]

1. Open the raw in Detail view and open the Edit panel.
2. Pick a Profile (Adobe Color is the default) and set Amount where offered.
3. Light: Auto or set the six tone sliders; check clipping with J; refine the Curve.
4. Color: set white balance (W sampler), then Color Mixer, Point Color, Color Grading.
5. Effects: Texture, Clarity, Dehaze, Vignette, Grain.
6. Detail: Sharpening; run Denoise for high-ISO files; consider Raw Details or Super Resolution.
7. Optics: enable lens corrections and chromatic aberration removal.
8. Geometry and Crop: Upright, Transform, crop; use Generative Expand if framing needs repair.
9. Local work: masks and the Remove tool.
10. Save a Named Version; export.

### D6. Masking \[S4\]

1. Open the mask tools and pick one (for example Select Sky or Brush).
2. Refine with Add, Subtract and Intersect; toggle the overlay with O.
3. Set sliders in the mask panel including Feather and Edge.
4. Rename the mask; use Duplicate and Invert to build the complementary edit.
5. Copy Edit Settings, then Paste to Entire Selection for batches.

### D7. Remove distractions \[S12\]

1. Choose the Remove tool (H).
2. Decide between Use generative AI, Detect objects, or both.
3. Circle, scribble or brush over the object; adjust Size; use Add and Subtract.
4. Generate; flip through the three variations; Refresh for another set.
5. Optionally switch on Visualize Spots (A) to find dust.
6. How a variation is committed or discarded is H.

### D8. Denoise a raw \[S14\]

1. Open a supported raw in Detail view.
2. Use Detail > Denoise or Photo > Enhance.
3. Set the amount; press and hold the preview to compare.
4. Click Enhance; find the new Enhance-NR DNG beside the original.
5. For batches select photos and press Ctrl+Alt+E (Windows) or Control+Option+E (macOS, see conflict 3).

### D9. Create and use presets \[S10\]

1. Open Presets (Shift+P); browse groups; step with arrow keys; set Amount from 0 to 200.
2. Star favorites.
3. Create Preset (name-only dialog); later choose Update With Current Settings.
4. Batch apply with Apply Preset on a selection.
5. Export an XMP to share.

### D10. Compare versions \[S3\]

1. Press Shift+V to open Versions.
2. Create Version (Shift+M macOS, Ctrl+Shift+S Windows) or Save as Named Version.
3. Hover rows to preview; open the Auto tab to see collaborator edits.
4. Rename or delete from the three-dot menu; Export with these settings.

### D11. Export \[S6\]

1. Select photos; click Share and pick JPG (Small), JPG (Large), Original, or Custom Settings (Shift+E).
2. Set File Type, Dimensions, Quality, watermark and More Options.
3. Click Export Photo and choose a destination.
4. Repeat later with Ctrl/Cmd+E.

### D12. Share an album \[S6\]

1. Select the album or photos and choose Share & Invite.
2. Get shareable link; set Link Access; invite emails and assign roles.
3. Use Customise and Settings tabs; click Done; manage later under Shared to Web.

### D13. Recovering from common failures

| Situation | Documented response | Src |
| --- | --- | --- |
| Photo blocked as duplicate | Rename the file outside Lightroom and add it again | S9 V |
| Edits missing after moving a Local file | Move the XMP sidecar with it; moves inside Lightroom do this automatically | S7 V |
| Cloud space pressure | Archive Photos locally or Archive Album Locally | S7 V |
| Need to work offline | Make Album Available Offline | S7 V |
| People View says No People Found Yet | Analysis may still be running on Adobe servers; closing the app does not stop it | S19 V |
| Face data privacy | Turn off Enable People View to delete face models | S19 V |
| Upscale scale exceeds model limit | Lower the scale or resize the image | S13 V |
| Cull decision is wrong | Mark as Select or Mark as Reject on the badge | S18 V |
| Contributed photo added by mistake | Delete it within 48 hours | S6 V |
| Sync errors, quota exceeded, missing Local files, restoring deleted photos | Not documented on opened pages | H |

## E. Appendix: gaps, guardrails and blueprint notes

### E.1 Hands-on verification backlog (prioritized)

Blocking for a faithful clone (P0):

1. Range, default and reset gesture for every global and local slider (Light, Color, Effects, Detail, Optics, Geometry, mask sliders).
2. Color panel: white balance dropdown and eyedropper, Temp, Tint, Vibrance, Saturation; any B&W channel mixer.
3. Window and Help menus, toolbar rail icons and order, left versus right panel placement, header bar controls.
4. Clone, Heal, Remove People, Remove Reflections, Remove Dust: overlays, sampling, result selection, commit and discard.
5. Remove tool: how a variation is committed or discarded.
6. Sync: conflicts, interrupted transfers, quota exceeded, missing files, relinking, pause and resume.
7. Local tab: albums, stacks, search, People and sharing behavior for Local items.
8. Undo and redo coverage; confirm History and snapshots are absent; before/after modes beyond Show Original.
9. Export: color-space list, AVIF and JPEG XL, overwrite handling, ppi, progress, failure and completion states, video export.
10. Crop: aspect presets, custom ratio entry, numeric fields, reset and confirm.
11. Technology Preview tab contents.

Important but not blocking (P1): 12. HDR and Pano merge dialogs; HDR Optimization settings. 13. Lens Blur controls and depth-map UI. 14. Preset import and export flows; creation dialog options; third-party presets. 15. Content Credentials, Edit in Photoshop and other-apps flows, Edit Videos. 16. Stacks, custom color labels, batch-rename template builder. 17. Subscription tiers (free or starter versus paid), credit allotments, plan limits on export. 18. Accessibility: focus order, screen-reader labels, keyboard-only operation. 19. Hover, focus, disabled, loading and error states for each component; notification and banner surfaces. 20. Deleted view retention window and restore flow. 21. Migration dialogs (Apple Photos, Classic), Apple TV and mobile preset sync.

### E.2 Classic look-alikes not documented for Desktop

The opened Desktop pages describe none of the following, so a competitor spec should not assume them from Lightroom Classic (absence is H, not proven): History panel and Snapshots, virtual copies, catalogs and collections, Library and Develop modules, Sync Settings, an import dialog with copy, move and add modes, per-setting checkboxes when creating a preset, Print, Book, Slideshow and Web modules, Publish Services. Desktop instead offers Versions, Cloud and Local tabs, albums and smart albums, Share & Invite, and Connections.

### E.3 Blueprint takeaways

1. Model two content roots (managed library and in-place folder view) behind one grid UI, with edit-storage rules per root.
2. Keep classical edits as a parameter stack with Versions as the checkpoint; treat enhancement and generative outputs as derived files stacked with their source.
3. Make masks the single local-editing primitive, with AI selections, range masks and brush as interchangeable inputs and one shared slider set.
4. For metered AI, show the cost before running, report progress in a persistent header, and land results in the filmstrip.
5. Gate GPU-heavy features on detected hardware and publish the floor (Adobe cites 8 GB dedicated or 16 GB shared on Windows, Apple silicon with 16 GB on macOS).
6. Ship biometric features such as People as opt-in, delete the model data when switched off, and plan region exclusions.
7. Export: a short preset menu, a custom screen, and a repeat-last shortcut; sync watermark settings across clients.
8. Treat Adobe's own docs as drifting: convert the conflicts in 0.6 and the backlog in E.1 into a conformance checklist before freezing requirements.

### E.4 Method and limits

All evidence is Adobe's published Desktop help (21 pages, last-updated dates in 0.5). No hands-on testing or screenshots were used, so visual design, spacing, animation and microcopy are out of scope. Several search queries returned dozens of locale duplicates of the same page and Classic or Web pages; those were discarded. Numeric ranges, defaults and states appear only where an Adobe page states them.
