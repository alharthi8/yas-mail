# Yas Mail

**Mail, made clear.**

A Windows email client built with Rust, Tauri 2, and a lightweight local web interface. Version 0.3.1 adds manual IMAP, POP3, and SMTP account settings alongside Google, Outlook, and Microsoft 365 / Exchange Online sign-in. Subtle interface animations respect reduced-motion preferences. Yas Mail's publisher configures the Google and Microsoft application identities once for the build; users do not register apps or enter client IDs.

## Open the app

Run `Yas-Mail-Setup.exe` to install for your Windows user, then open **Yas Mail** from Start. The installer creates the Start menu shortcut and repairs it on updates if it is missing. No administrator access is required. For a portable launch, run `Yas-Mail-0.3.1.exe` from the delivery folder; the portable app does not add a Start menu entry.

Close an older running Yas Mail window before launching the updated version. Both installed and portable builds use the Windows GUI subsystem, including this debug preview, so launching the app does not open a command window. The app opens a clearly labeled demo workspace until you connect an account. No demo action sends real email.

Choose **＋** beside the account tabs. Use a provider button for Gmail, Outlook, or Microsoft 365 / Exchange Online, or choose manual setup for another IMAP, POP3, or SMTP server. See [SETUP.md](SETUP.md) for account and local-AI help, [MANUAL-ACCOUNTS.md](MANUAL-ACCOUNTS.md) for server settings, and [CLOUD-DRIVES.md](CLOUD-DRIVES.md) for files from your drives. The publisher's one-time OAuth setup is in [PUBLISHING.md](PUBLISHING.md).

The current preview has no publisher Google/Microsoft credentials provisioned. Those provider buttons show that sign-in is unavailable in this build; manual setup and the demo remain separate options. Enabling browser sign-in requires the publisher to supply the registered application identities and rebuild. Manual server settings do not bypass a provider's authentication policies.

## Features

Available features depend on the account type. Google/Microsoft accounts provide the integrated mail, contacts, and calendar functions; manual accounts follow the protocol limits in [MANUAL-ACCOUNTS.md](MANUAL-ACCOUNTS.md).

- A frameless window with a compact integrated header. The native title bar is disabled (`decorations: false`); minimize, maximize/restore, and close controls are part of Yas Mail's header and remain available in dialogs. Drag the header to move the window or double-click its drag area to maximize/restore.
- Chrome-style account tabs across the top replace the sidebar account selector. Each connected account has its own avatar, color, and email label; the top-row plus button adds an account.
- Simple Google/Microsoft sign-in buttons, browser consent, clear pending/error states, and no end-user application-ID fields. Subtle dialog, content, and button transitions honor the system's reduced-motion preference.
- Microsoft 365 / Exchange Online uses Microsoft's modern browser sign-in. On-premises Exchange can be connected through administrator-enabled IMAP or POP3 with SMTP; native EWS, MAPI/HTTP, and ActiveSync integration are not included.
- Manual IMAP + SMTP, POP3 + SMTP, and SMTP-only accounts, with custom hosts, ports, usernames, required TLS or STARTTLS, connection testing, and credentials protected by Windows Credential Manager. POP3 leaves messages on the server. SMTP-only accounts send without an inbox.
- Drag account tabs left or right to reorder them, with a live preview and automatic scrolling near the strip edges. The order is saved on this device and restored when Yas Mail reopens. Reordering keeps the active account unchanged. Escape cancels; Ctrl+Shift+Left/Right Arrow moves the focused tab using the keyboard.
- Drag a connected account tab away from the tab row and release to open it in its own Yas Mail window. The toolbar also has Open account in new window and Return account to main window buttons. Detached windows keep the account's view and appearance and use the same saved normal accounts and preferences. Detaching the last account from a detached source closes that emptied source; the main window stays open.
- Drag a tab onto the main window's tab strip or another open Yas Mail window's tab strip to move it into that window. A blue marker previews the insertion gap. The current folder, message, avatar, and color move with the account; an emptied detached source window closes, while the main window stays open. A destination with an open modal cannot receive a tab. Escape cancels; releasing elsewhere still creates a new window. Transfers work between windows in the same running Yas Mail app session, not separately launched app processes.
- Each connected account belongs to one open Yas Mail window at a time. Account-scoped native commands check the calling window's ownership. Closing a window returns its accounts to the main window or another remaining Yas Mail window, without disconnecting them. Closing the last window exits; saved accounts return to the main window on the next launch.
- Tabs keep their last section, folder, search, and message selection in memory while Yas Mail is open. Arrow keys, Home, and End navigate focused tabs; Ctrl+Tab and Ctrl+Shift+Tab switch accounts in the native app.
- Per-account initials/emoji avatar, accent color, email signature, and optional writing-style memory.
- The new Wing Letter identity matches Yas Browser's royal-blue family: a white envelope, swept wing, and sky-blue feather on a rounded tile. Assets include editable SVG, transparent PNG, a Dubai wordmark, and nine Windows icon sizes from 16 to 256 pixels. The font is used from the installed system and is not bundled. Per-account colors remain distinct.
- Inbox, starred, sent, drafts, archive, junk, and trash; provider search and paginated mail loading.
- Read/unread, star/unstar, archive, trash, reply, reply all, forward, compose, and provider draft creation.
- Send attachments (9 MB total for Gmail, 3 MB for Outlook) and save received file attachments using a Windows save dialog.
- Connect Google Drive, OneDrive, Dropbox, or Zoho WorkDrive through browser OAuth. Browse folders, search files, attach a downloaded copy to the current email, or insert an existing provider link. Cloud connections are separate from mailbox sign-in; see [CLOUD-DRIVES.md](CLOUD-DRIVES.md).
- Primary calendar month view, agenda, event/meeting creation with attendee invitations, and event deletion.
- CSV and vCard contact import/export, with preview, duplicate skipping, and partial-import reporting.
- Contact health: missing names/phones/emails, email and phone checks, duplicate comparison, suggested names, formatting cleanup, and non-destructive primary-detail consolidation.
- Read, create, edit, and delete contacts at the provider; recipient suggestions from contacts.
- Provider-side sender-to-trash rules that operate even when Yas Mail is closed; rule removal.
- Light/dark/Windows theme, text size, density, right/bottom/individual reading view, refresh interval, startup section, and calendar week start.
- Send/trash confirmation preferences, automatic mark-read preference, keyboard shortcuts.
- Local reply templates, basic rule-based writing checks, and WebView spelling support.
- Local SLM rewriting with tone selection, preview/apply controls, and opt-in style examples.

## Important preview boundaries

Live Google/Microsoft authentication, mail, contacts, calendar, and rules require Yas Mail's publisher-configured OAuth registrations and user consent. They have not been verified against a real mailbox in this build. A provider or organization may require admin approval. Dropbox and Zoho WorkDrive retain manual application setup in this cloud-drive preview; Google Drive and OneDrive use the build's publisher configuration.

Manual mail accounts expose only the features supported by their protocols. They do not synchronize contacts, calendars, meeting invitations, or sender rules. POP3 provides inbox reading with messages left on the server; SMTP alone provides sending. This release does not add native on-premises Exchange collaboration features. See [MANUAL-ACCOUNTS.md](MANUAL-ACCOUNTS.md) for the precise account boundaries.

Cloud drives currently provide a read-only file picker for composing mail. Full background drive synchronization and upload management remain future work. **Attach copy** fetches the chosen file into the current message within the email attachment limit. **Insert link** uses an existing link and preserves the provider's access rules, so recipients may still need permission. Yas Mail does not create public links or change sharing permissions, and sending remains an explicit action.

Sender rules are **not the providers' native blocked-sender lists**. Gmail filters and Outlook inbox rules move future matching mail to trash/deleted items. They do not alter existing mail or create native junk-sender entries.

Mail is rendered as plain text. HTML-only Gmail mail shows a plain-text-unavailable notice; external images and scripts do not load. Reading-pane reply/forward creates a new composed message, rather than a provider-native threaded reply. Existing draft editing, rich-text signatures, custom folders, unified inbox, offline mailbox cache, push sync, reminders, recurring-event editing, RSVP, and automatic Teams/Meet links are not included yet. The calendar displays up to 250 events per requested period. Standard file attachments are supported; embedded Outlook item/reference attachments are not.

Refresh runs only while the app is open, for the current section. It does not run as a Windows background service. AI runtime/model are separate local dependencies. Style memory is example-based personalization, not model training.

Version 0.2.0 passed eleven native pointer checks with synthetic accounts at 225% display scaling, 37 JavaScript tests, 38 Rust tests, a separate Windows credential round-trip, and six cloud UI workflows with mocked providers. The 0.1.6 crowded-receiver scrolling check remains a historical result. Details and limits are recorded in [VALIDATION.md](VALIDATION.md). Live connected accounts, mixed-DPI monitors, and manual interaction feel remain unverified.

## Build from source

Install Rust stable (MSVC), Visual Studio Build Tools with Desktop development with C++, the Windows SDK, and WebView2. Node/npm are not required for the frontend.

```powershell
cd src-tauri
cargo test --locked
cargo run --locked
cargo build --release --locked
```

With the Tauri CLI installed, run `cargo tauri build` from this project to produce an NSIS installer. The compiled app uses the static files in `ui/`. Set the publisher OAuth build variables described in [PUBLISHING.md](PUBLISHING.md) to enable Google/Microsoft sign-in. A source or demo build without them keeps those providers unavailable rather than asking users for developer credentials.

## Data

OAuth access/refresh tokens, Google desktop client metadata, manual-account credentials, and short style examples are protected by Windows Credential Manager under the legacy service `GoodMail`. Public account metadata is in the Tauri app-data directory. Normal Yas Mail windows share saved connections, appearance, UI preferences, and account-tab order. UI preferences and order use WebView local storage; saved order contains account IDs only. Per-tab view state, window ownership, and loaded live mailbox/contact data are kept only in memory; this does not add an offline mailbox cache or restore the previous window arrangement after exit. Draft content and contacts are not sent to an AI service automatically.

Cloud sign-in happens in your browser, with credentials managed by the native Windows app. Browsing fetches provider file metadata; selecting **Attach copy** fetches the selected file for that message. An inserted link keeps its existing access rules. Connecting a drive or preparing a file does not send the message. See [CLOUD-DRIVES.md](CLOUD-DRIVES.md) for provider-specific setup and connection details.

Sending is not retried automatically, avoiding accidental duplicate messages after an ambiguous network failure. “Accepted” means the provider accepted the send request, not that delivery is guaranteed.

Compatibility: the internal app identifier, credential-service namespace, and local storage keys retain their legacy values so the rename preserves connected accounts, saved signatures, avatars, colors, and preferences. All visible branding is Yas Mail.

## Signed in-app updates

Install this updater-enabled preview once. Both the startup check and Settings → Check for app updates use GitHub Releases. Approve the update, then fully quit and reopen when convenient. Downloads are verified with the embedded public key before replacing the app executable. The active session and user-data folders are left intact. See [UPDATES.md](UPDATES.md) for release/key management and full-installer limitations. Windows Authenticode publisher signing and macOS updates are not configured.
