# CAPTURE: Definitive In-Game Overlay & Replay System
**Complete Agent Implementation Addendum** (received from the user 2026-11-08; replaces every previous overlay instruction)

## Mission
Build a polished, exceptionally simple in-game overlay and replay system for CAPTURE.
Combine the most useful capabilities of Steam Game Recording and AMD Adrenalin, improve their usability, and eliminate unnecessary steps.
Do not blindly reproduce every feature from either application.
Our goal is not to create another video editor or complicated recording dashboard. Our goal is to make recording, reviewing, bookmarking, and saving gameplay almost effortless.
This addendum replaces every previous overlay instruction. It extends the existing CAPTURE design and must preserve all established recording features, hardware compatibility, audio separation, and storage functionality.

## 1. Fundamental Design Philosophy
CAPTURE should be invisible until needed.

During normal gameplay:
- Background recording runs automatically according to user preferences.
- No permanent HUD, timer, or recording indicator.
- Hotkeys work without opening the overlay.
- Optional short audio notifications confirm actions.
- No unnecessary popups or interruptions.

When the overlay is opened:
- Immediately show recording status.
- Make the most common actions obvious.
- Provide instant access to previous gameplay.
- Allow quick microphone and audio adjustments.
- Allow access to saved clips and recording locations.
- Provide a clear path to the full Windows settings application.

The overlay must never feel like a second desktop application squeezed on top of a game.
Prioritize simplicity, consistency, speed, and reliability over feature quantity.

## 2. The Main Overlay
Create a clean, compact, modern interface with four primary actions.

**Primary actions**
- **Save Replay**: Instantly save the previous configured duration of gameplay to a permanent, editor-ready video file.
- **Add Marker**: Bookmark the current gameplay moment for later review without necessarily creating a full video clip.
- **Review Recording**: Open a simple timeline showing recently recorded gameplay, including bookmarks and available footage.
- **Start / Stop Recording**: Control full-session recording independently from the background replay buffer.

These four actions must be immediately visible.

**Additional information.** Display a small, readable status area containing:
- Current game or capture target.
- Recording or buffering status.
- Available replay duration.
- Selected microphone and live input level.
- Recording storage location.
- Remaining disk space, accessible without clutter.

Include a small recent-clips area with thumbnails and a direct way to open the clip library or recording folder.
Do not fill the main overlay with bitrate graphs, codec information, multiple audio mixers, or advanced technical details.

**Example hierarchy**
```
CAPTURE
Recording status: Buffering, last 5 minutes available
[ Save Replay ] [ Add Marker ]
[ Review Recording ] [ Start Recording ]
Microphone: Selected Device | Live Level | Mute
Recent Clips: Three recent thumbnails
[ Quick Settings ] [ Open Folder ] [ Full Settings ]
```
This is a layout concept, not a mandatory visual implementation. Improve it wherever actual usability testing suggests a better arrangement.
Use appropriately large buttons, clear labels, and restrained visual styling. Avoid needless nested menus.

## 3. Instant Replay
Instant replay is a primary feature, not a secondary recording mode.

**Requirements**
- Continuously maintain a rolling recording buffer when enabled.
- Save the previous configured duration using a dedicated hotkey.
- Allow replay durations such as 30 seconds, 1, 2, 5, 10, or 20 minutes, plus custom values.
- Allow the user to change the replay duration without restarting the application where technically feasible.
- Never interrupt background capture when saving a replay.
- Save the replay directly to the configured permanent clip folder.
- Preserve independently recorded audio tracks.
- Show a brief optional confirmation when saving succeeds.
- Allow immediate access to the resulting file.

**Direct-to-file behavior.** Saved clips should be ordinary, editor-ready files, preferably MP4 with supported multitrack audio. No manual export process should be required. If a temporary recording format is needed for safety or crash recovery, automatically finalize the clip into the preferred output format. Use lossless remuxing when possible. Avoid unnecessary full-video re-encoding. Provide accurate feedback if a clip is still being finalized rather than incorrectly claiming it is ready.

**Replay buffer.** Allow configurable:
- Maximum replay history duration.
- Maximum temporary disk usage.
- Minimum free disk space to preserve.
- Buffer location and drive.
- Automatic overwrite of the oldest unprotected footage.

Never automatically delete permanent saved clips.

## 4. Simple Live Recording Timeline
This is one of our highest-priority improvements.
While gameplay recording continues, the user must be able to open the overlay and immediately review previously recorded footage.
The timeline is a replay tool, not a video editor.

**Required controls**
- Play and pause.
- Scrub backward and forward.
- Seek by clicking or dragging.
- Jump to the beginning or current live position.
- Jump between manual markers.
- Preview footage near a selected position.
- Select a section of footage.
- Save the selected section as a clip.

Keep timeline controls small, understandable, and responsive.

**Simple clip creation.** The workflow should be:
1. Open the replay timeline.
2. Find the moment.
3. Press Create Clip.
4. Adjust two simple start/end handles if needed.
5. Press Save Clip.

The saved file should immediately appear in the appropriate game folder after finalization.
Provide a default selection around the current playhead, reducing the need to adjust handles manually.
Allow clicking a marker to start a clip selection around that moment.

**What NOT to build:** multilayer video timelines, transitions, special effects, color grading, advanced audio editing, keyframe animation, complex rendering settings, video compositing, full editing workspaces. Those capabilities belong in FrameCut or another dedicated video editor. The overlay should only find, preview, select, and save footage.

## 5. Marker and Bookmark System
Implement simple, reliable gameplay bookmarks inspired by Steam's marker system. Markers should help users return to important moments without immediately creating a permanent video clip.

**Marker hotkey.** Provide a configurable global hotkey. When pressed:
1. Record the current gameplay timestamp.
2. Add a visible marker to the session timeline.
3. Play a short optional confirmation sound.
4. Continue gameplay without opening any interface.

The operation should feel instantaneous.

**Notification sound.** The confirmation sound should:
- Be short, subtle, and recognizable.
- Have adjustable volume.
- Be individually enabled or disabled.
- Use a selectable playback device where practical.
- Be excluded from CAPTURE's recorded digital audio tracks.

Design the audio architecture so CAPTURE-generated notification sounds do not enter the game, voice-chat, or mixed recording tracks. Where the selected capture method cannot reliably exclude the sound, prefer silent confirmation rather than contaminating recordings. Document the unavoidable limitation that a physical microphone may pick up sound played through speakers.

**Marker features**
- Show markers directly on the replay timeline.
- Click a marker to jump to its timestamp.
- Navigate to previous or next marker.
- Add a short optional name or note.
- Rename or remove markers.
- Show marked moments in the session library.
- Preserve marker metadata with the corresponding recording session.

Keep all naming and editing optional. A marker should require only a single hotkey press.

**Marker protection.** Implement optional automatic protection of footage surrounding a marker. Allow users to configure the protected duration before and after the marked moment. Enable a sensible, storage-limited protection preset by default, but make this configurable. If storage limits prevent protection, communicate this accurately. Differentiate clearly between:
- Bookmark exists.
- Footage is temporarily buffered.
- Footage is protected from overwrite.
- Footage has been permanently saved.

Never imply that a marker guarantees permanent footage when it does not. Avoid complicated marker tagging systems or automatic game-event integrations in the initial implementation.

## 6. Audio and Microphone Controls
Audio selection should be exceptionally straightforward.

**Main overlay audio area.** Show: currently selected microphone; live microphone activity meter; microphone mute toggle; quick microphone dropdown; compact access to audio source settings. Selecting a different microphone should be quick and intuitive.

**Quick audio settings.** Provide one small panel for: microphone input selection; game audio source selection; voice-chat application selection; audio playback/output device selection; individual audio source enable/disable toggles; basic independent volume adjustments; microphone test.

Avoid exposing technical audio routing terminology unless advanced settings are opened. Use friendly descriptions such as "Your microphone", "Game sound", "Voice chat", "Other computer audio".

**Microphone testing.** Display live input levels. Optionally record a short test sample. Play the sample through the user's selected device. Identify muted, disconnected, or silent microphones. Prevent accidental monitoring loops or feedback.

**Separate audio tracks.** Preserve the existing requirement for independently editable audio streams. Recommended recording layout:
- Video: Gameplay footage.
- Audio 1: Combined playback mix, where useful.
- Audio 2: Game audio.
- Audio 3: Microphone.
- Audio 4: Voice chat.
- Additional tracks when configured and supported.

Keep timestamps synchronized. A supported video editor should be able to access the individual tracks without trying to separate previously mixed audio. If an editor cannot import every embedded track correctly, provide a simple export option for separately named, synchronized audio files. Do not assume that independently capturing voice chat is possible when a game already mixes voices into its own output.

**Device reliability.** Handle microphone disconnection, headset changes, default Windows device changes, audio application restarts, device format changes, silent or unavailable capture sources. Where possible, recover automatically. Otherwise clearly identify the problem.

## 7. Quick Settings Versus Full Windows Settings
Preserve the existing full Windows settings application. Do not remove, replace, or unnecessarily redesign it. The overlay is a quick-access interface. The Windows application remains the complete configuration center.

**Overlay quick settings** (only the most frequently changed options): replay duration; microphone selection and mute; game and voice-chat audio source selection; recording on/off; basic recording quality preset; current recording destination; marker notification settings; open full settings.

**Full Windows settings.** Retain and expand the existing detailed settings where necessary.
- *Recording*: background recording mode; automatic recording and game detection; manual recording mode; capture resolution and frame rate; quality presets and custom bitrate; video codec and encoder selection; recording format; replay duration and buffer behavior; per-game recording profiles.
- *Audio*: all input and output devices; source routing; separate audio tracks; individual volume settings; monitoring and microphone testing; audio quality and synchronization; notification sound routing.
- *Storage*: fully configurable root recording location; independent optional paths for buffers, clips, sessions, and screenshots; automatic game folders; file naming conventions; storage limits; free-space reserve; cleanup behavior; optional migration when changing locations.
- *Hotkeys*: open or close overlay; save replay; add marker; start or stop recording; mute microphone; screenshot; other useful shortcuts. Provide hotkey-conflict detection and simple reassignment.
- *Overlay*: enable or disable overlay; overlay hotkey; size and opacity; optional notifications; notification sound and volume; interface scaling; accessibility settings.
- *General*: startup behavior; system tray operation; default game detection; hardware compatibility; troubleshooting and diagnostics; updates.

Advanced controls should be available but not forced upon ordinary users.

**Synchronization.** Settings changed in the overlay must appear in the Windows application, and vice versa. There must be one authoritative configuration system, not competing settings stores. Apply changes immediately where safe. Clearly indicate when a recording restart or new session is required.

## 8. Storage and File Organization
Preserve CAPTURE's automatic folder management and complete location configurability. Users must be able to choose their own drives and directories. Example: `D:\Captures`.

Automatically organize: clips by game; full recordings by game and session; screenshots by game; temporary replay data in a designated buffer directory. Use understandable filenames containing a timestamp and optionally the game name.

**Quality-of-life requirements**
- Open the recording folder with one click.
- Open a clip's actual location from the overlay.
- View available disk space.
- Identify how much space temporary footage consumes.
- Clearly distinguish temporary and permanent footage.
- Keep markers associated with the appropriate sessions.
- Preserve saved clips during buffer cleanup.
- Never move existing recording files silently when the user changes destinations.
- Validate new destinations before writing.
- Handle missing, disconnected, or full drives gracefully.

Provide an optional Copy File Path action. Avoid hiding actual recordings inside a proprietary media library. The filesystem should remain useful even without CAPTURE running.

## 9. Recent Clips and Library
Keep the overlay's library deliberately simple. Show a small number of recent clips with: thumbnail; game name; date and time; clip duration; playback action; open file location; delete action with confirmation. Offer a View All option. The full Windows application can provide a richer library with searching, sorting, game filters, and session browsing.

**In-game library behavior.** Opening recent clips must not stop background recording. Allow quick preview without launching another application where practical. Provide clear playback controls and a direct return to the overlay home screen. Avoid social-media sharing interfaces, cloud accounts, or complicated library management tools in the initial overlay.

## 10. Recording Feedback and Notifications
CAPTURE should provide feedback without disturbing gameplay. Support optional temporary notifications for: replay successfully saved; marker successfully added; recording started; recording stopped; microphone disconnected; capture failure; storage becoming full.

Notifications should: be brief; be individually configurable; never remain onscreen permanently; be disabled completely if desired; not contaminate the recording when supported.

Do not interrupt gameplay with unnecessary confirmation dialogs. Destructive actions such as deleting permanent files must still require appropriate confirmation.

## 11. Supported Hardware and Games
Preserve cross-vendor compatibility. Support available encoding backends including NVIDIA NVENC, AMD AMF, Intel Quick Sync, and a software encoding fallback. Detect supported hardware and choose a sensible default. Do not make any overlay feature unnecessarily dependent on NVIDIA, AMD Adrenalin, Steam, or a specific game launcher. Target Steam, Epic, Xbox PC, standalone games, and normal application-window capture where supported.

**Overlay compatibility.** Use supported Windows APIs and non-invasive overlay methods. Do not assume that one overlay approach will work with every exclusive-fullscreen game or anti-cheat environment. Test borderless, windowed, and fullscreen behavior. When the overlay cannot open over a game, provide a graceful fallback using the desktop application. Core recording and hotkeys should continue functioning independently of overlay availability wherever supported. Never sacrifice recording reliability simply to display the overlay.

## 12. Performance and Reliability
The recording engine must remain independent of the overlay interface. Opening the overlay, previewing footage, navigating markers, or saving clips must not interrupt active recording. Requirements:
- Avoid unnecessary background rendering.
- Suspend overlay rendering when hidden.
- Avoid creating duplicate capture or encoding pipelines.
- Use available recording segments for timeline playback.
- Prevent temporary segment cleanup while footage is being previewed or exported.
- Use safe file finalization and crash recovery.
- Keep memory and GPU resource usage controlled.
- Benchmark performance overhead against recording with the overlay closed.
- Gracefully handle rapid overlay opening and closing.

Prioritize stable capture, correct audio, and file integrity over sophisticated animations. The interface should feel smooth and responsive without excessive visual effects.

## 13. Additional Quality-of-Life Features
Include these where they provide measurable everyday value without complicating the interface.

**High priority**
- One-click save around a marker: click a marker and save a configurable amount of footage around it without adjusting timeline handles.
- Quick replay presets: offer a small selection of common clip lengths directly in the overlay.
- Recent clip shortcut: after saving a clip, provide a quick way to open it or its folder.
- Smart device recovery: reconnect to preferred audio devices where possible after disconnection.
- Per-game preferences: allow games to inherit global settings or use an easily configured override.
- Clear failure reporting: explain exactly what is not working and suggest a practical correction.
- Instant screenshots: a single hotkey saves a screenshot into the correct game directory.

**Optional later enhancements:** controller shortcuts for clipping and markers; user-selected favorite clips; optional thumbnail previews when hovering on a timeline; lightweight clip naming; configurable pre-marker and post-marker protection; direct "Open in FrameCut" action; portable marker/session metadata for future editor integration.

Do not build speculative automation, elaborate game-event detection, cloud sharing, or AI-assisted editing until the core experience is reliable and proven.

## 14. Visual Design Requirements
The overlay should look like a premium modern desktop application. Design principles: minimal layout; clear visual hierarchy; consistent spacing; legible typography; large primary controls; restrained accent colors; subtle animation; excellent contrast; appropriate scaling for different resolutions.

Prefer a compact overlay occupying only the necessary screen area. Allow the replay viewer to expand when actually reviewing footage. Do not permanently dedicate a large portion of the overlay to timeline editing controls. Do not add unnecessary navigation sections or decorative information. The interface should feel instantly understandable even to someone who has never used recording software.

## 15. User Workflows to Optimize
- **A: Save a moment.** User is playing a game. Presses the save replay hotkey. A clip is created directly in the configured folder. No further action required.
- **B: Remember something important.** User presses the marker hotkey. A short confirmation sound plays. Gameplay continues. Later, the user opens the replay timeline, clicks the marker, and watches the moment.
- **C: Create a precise clip.** User opens the overlay. Selects Review Recording. Scrubs to the desired moment. Uses the two simple trim handles. Presses Save Clip. The video is ready for editing.
- **D: Fix microphone selection.** User opens the overlay. Selects the microphone dropdown. Chooses the correct device. Sees the live audio meter respond. Closes the overlay and resumes gameplay.
- **E: Change storage location.** User opens quick settings or the full Windows settings window. Chooses a new folder. CAPTURE validates the destination and uses it for future recordings. Existing files remain untouched unless the user explicitly requests migration.
- **F: Edit a saved clip.** User opens a saved MP4 in a compatible video editor. Video, microphone, game audio, and independently captured voice chat are available as separate synchronized streams. No audio separation processing should be required for streams that were recorded independently.

## 16. Development and Validation
Inspect the existing CAPTURE implementation before making changes. Preserve working systems. Do not conduct an unnecessary full rewrite or introduce a disconnected second application. Implement the smallest dependable architecture that supports the required behavior.

**Implementation order**
1. Verify the existing recording engine and replay-buffer interfaces.
2. Implement manual markers and persistent timeline metadata.
3. Implement reliable live replay navigation.
4. Implement simple timeline clip selection and saving.
5. Implement the invisible hotkey-activated overlay.
6. Connect quick recording controls and microphone selection.
7. Connect recent clips, storage information, and Windows settings.
8. Implement notification sound isolation and device recovery.
9. Validate compatibility, performance, and failure handling.
10. Polish the interface after core workflows have passed.

Each milestone should remain testable and independently useful.

**Mandatory acceptance tests**
- Recording continues with the overlay open and closed.
- A single hotkey creates a playable, editor-ready clip.
- A marker hotkey creates an accurately timestamped bookmark.
- Confirmation sounds do not enter recorded digital tracks under supported configurations.
- The live timeline can play existing footage while recording continues.
- Two-handle trimming saves the intended footage range.
- Marked footage is protected when marker protection is enabled.
- Saved clips retain their separate audio streams.
- Changing microphones updates the actual capture source.
- Disk limits never cause accidental deletion of permanent clips.
- File locations remain user-configurable.
- AMD, NVIDIA, and Intel backend support is verified on available hardware, with untested combinations explicitly identified.
- Opening, closing, and using the overlay does not introduce unacceptable measured performance regressions.
- Unsupported fullscreen and overlay situations fail gracefully.
- The Windows settings application continues functioning.

Do not claim hardware-specific compatibility without testing the relevant hardware.

**Agent completion report.** At each development milestone provide: features actually implemented; real test results; screenshots of the current interface; performance measurements where relevant; known limitations; remaining work. Distinguish implemented behavior from prototypes, placeholders, and future enhancements.

## Final Product Standard
CAPTURE should feel like a recording system that does its job automatically and quietly. When the user wants something, it should be immediate:
- Save a moment: One hotkey.
- Bookmark a moment: One hotkey.
- Review gameplay: Open overlay, select Review.
- Create a clip: Select the moment and save.
- Change microphone: Open overlay and select a device.
- Find a video: Open the recent clip or its folder.
- Change advanced settings: Open the existing Windows settings application.

No unnecessary steps. No feature bloat. No complicated editing interface. No proprietary barriers between the user and their recordings. No hardware-brand lock-in.
Build the simplest interface that fully supports the useful functionality, not the interface with the most functionality visible.
The final result should be easier to learn and more convenient to use than Steam Game Recording, AMD Adrenalin, or OBS for ordinary gameplay capture.
