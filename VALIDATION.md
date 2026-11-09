# Yas Mail preview validation

## Version 0.3.0 checks

This release adds native manual IMAP/SMTP, POP3/SMTP, and SMTP-only accounts, a two-step setup with connection testing, and editable server settings. Passwords are stored separately in Windows Credential Manager; public account metadata contains no passwords. Existing OAuth account files remain compatible.

Microsoft 365/Exchange Online uses the existing Microsoft browser sign-in. An on-premises Exchange server can be used through IMAP or POP3 plus SMTP when its administrator enables those protocols. EWS, MAPI, ActiveSync, NTLM, and manual OAuth-token entry are not implemented. Publisher registrations for Google/Microsoft sign-in remain unconfigured in this preview; manual protocol connections work independently.

Native and UI capability checks keep unsupported contacts, calendars, rules, POP3 mutations/search/drafts, and SMTP inbox operations out of the protocol accounts. Incoming connections require verified TLS or STARTTLS, POP3 never issues DELE, connection testing never sends mail, and SMTP sending has no automatic retry. See `MANUAL-ACCOUNTS.md` for exact limits.

Current checks and final build provenance are recorded in `manual-accounts-validation.json` and `release-validation.json`. Earlier sections are historical results for their stated versions. No real external mail server or live mailbox was used for validation.

- All 67 Rust tests passed; the two existing opt-in Windows credential smoke tests were intentionally ignored in this run. Twelve focused protocol tests cover configuration, MIME and transfer limits, stable identifiers, Bcc, and protocol behavior.
- Three positive encrypted loopback transcripts passed for IMAP, POP3, and SMTP. They verify IMAP fetch selection amid unsolicited updates, non-destructive POP3 commands, and SMTP authentication/NOOP without MAIL or DATA during testing. Two refusal transcripts prove IMAP/POP3 do not transmit credentials if required STARTTLS is rejected.
- Synthetic test certificates are scoped to private test connectors. The Windows trust store was not changed; no certificate-verification bypass is exposed in the product. The reproducible fixtures and their README are in `src-tauri/tests/fixtures`.
- All 52 JavaScript tests and seven production-UI workflows passed. Mocked native commands verified shared/separate SMTP credentials, an encrypted no-auth relay, test-before-connect, persistent account identity during edits, POP3 and SMTP capability gates, accepted-send warnings, late-result/password cleanup, and the scrolling form/footer in dark mode at 1244×723. The UI checks used no real socket, account, credential, or send; native protocol tests used loopback only.
- The Windows executable and installer built as Yas Mail 0.3.0 with GUI subsystem 2 and matching source hashes. Installation exited 0. The installed app was opened and the Microsoft 365/Exchange Online choice and manual setup verified. A synthetic local-only email value was used to open the server step; the incoming/SMTP host, port, TLS, credential controls, and sticky Test/Connect footer were present at 225% display scaling. The form was closed and discarded without testing or saving any account. Packaging verifies the installed binary, shortcuts, and absence of console children.

## Version 0.2.2 checks

This release moves Google/Microsoft OAuth application configuration into the native build. Users choose Google or Microsoft without entering application IDs or secrets. Google Drive and OneDrive use the same publisher registrations with separate consent. Dropbox and Zoho WorkDrive retain their manual preview setup.

The delivered preview has no publisher registrations configured. The interface shows those providers as unavailable and does not open a broken or simulated sign-in. Provisioning Yas Mail's publisher registrations and testing live accounts remain outstanding. See `PUBLISHING.md`.

Mailbox sign-in supports cancelling while waiting for the browser callback. After accepting the callback, the app finishes sign-in; cancellation does not revoke permissions at the provider. Saved mail credentials now use bounded Windows Credential Manager chunks with legacy read compatibility. The app also adds short modal, content, picker, and control animations with a reduced-motion override; account-tab drag geometry is unchanged.

The 0.2.2 automated checks, source hashes, publisher-readiness state, and Windows build/install results are recorded in `publisher-signin-validation.json` and `release-v022-validation.json`. All results in the older sections below apply to their stated versions.

- All 46 JavaScript tests passed, including provider readiness, provider-only commands, pending/cancel guards, and credential-free sign-in markup.
- All 53 ordinary Rust tests passed, with two Windows credential smoke tests intentionally ignored. The new long-mail-credential smoke ran separately and passed 1/1 using only a synthetic unique entry with cleanup. Publisher configuration, legacy credential migration/recovery, and per-window cancellation are covered by the ordinary tests.
- Five production-UI workflows passed with mocked native OAuth commands at 1244×723: configured provider buttons; pending and acknowledged cancellation; accepted-callback cancellation and reduced-motion styles; Google/OneDrive provider-only cloud requests; and unavailable-provider light/dark layout. No live accounts, provider requests, or browser OAuth were used.
- The design preview `Yas-Mail-sign-in-preview.png` uses synthetic configured-provider availability. It illustrates the UI, rather than claiming the delivered preview has provisioned sign-in.
- The executable and NSIS installer built as 0.2.2 with Windows GUI subsystem 2. All 34 runtime inputs matched the final build manifest. The update installer exited 0, and the installed application was opened: the provider buttons showed the native unavailable status, there were no client-ID or secret fields, and the demo/back control remained accessible. The installed binary, shortcuts, and absence of console children were verified during packaging.

## Version 0.2.1 checks

This patch addresses Microsoft `unauthorized_client` caused by entering an email address as the application client ID. Mailbox and cloud-drive connections now validate IDs in both the interface and Rust before starting OAuth. Outlook shows Microsoft-specific setup instructions, the personal-account audience requirement, and no Google secret field. Google shows its desktop-client instructions. Format validation does not verify an application's existence, audience, permissions, or consent.

- All 40 JavaScript tests passed, including three new client-ID and setup-guidance checks.
- All 42 Rust tests passed, including four new shared client-ID tests. The existing Windows Credential Manager smoke test remains intentionally ignored in this run; its separate 0.2.0 result is below.
- Four targeted production-UI workflows passed with mocked native OAuth commands: Outlook rejects an email locally, a synthetic Microsoft GUID reaches the command with an empty secret, switching to Google shows its desktop fields and rejects an email, and OneDrive applies the same Microsoft checks. See `oauth-setup-validation.json`. These checks made no real OAuth or provider calls.
- The Windows executable and NSIS installer built as 0.2.1 with GUI subsystem 2; the update installer exited successfully. Runtime source hashes, installed executable, shortcuts, and console-child checks are recorded in `release-v021-validation.json`. The installed Google and Outlook forms were visually checked. No live sign-in is claimed.

The 0.2.0 cloud, icon, native drag, credential-storage, and installation results below are a historical baseline for this patch, not fresh 0.2.1 end-to-end results.

Version 0.2.0 adds the **Wing Letter** identity, inspired by Yas Browser's royal-blue family: a white envelope, swept wing, and sky-blue rear feather. The SVG/PNG assets, Dubai wordmark, and Windows ICO with nine sizes from 16 through 256 pixels share that identity. No font files are bundled.

This release adds browser-OAuth cloud connections for Google Drive, OneDrive, Dropbox, and Zoho WorkDrive. The read-only email file picker browses folders, searches files, fetches selected downloadable copies into the current message, and inserts existing provider links. It preserves existing access rules and leaves sending as an explicit user action. Background drive synchronization and upload management are outside this release's scope. Setup is documented in [CLOUD-DRIVES.md](CLOUD-DRIVES.md).

Legacy app identity, credential-service identifiers, and local storage keys remain internal compatibility values so saved accounts and preferences survive the rename. Native window events use the coordinated `yasmail:` namespace.

## Version 0.2.0 checks

- All 37 JavaScript tests passed. All 38 ordinary Rust tests passed; one Windows credential test is intentionally ignored by the ordinary run. That test was run separately and passed, round-tripping a large synthetic credential across 17 Windows Credential Manager chunks and removing its test entries.
- Six browser workflows passed against the production UI with a synthetic native/provider bridge: all four connection forms and Zoho regions; disconnect and credential handling; folder/search/pagination navigation while preserving the draft; mixed local/cloud attachments and aggregate size caps; existing links and link errors; and pending-operation/closed-draft guards. These checks made no real OAuth or provider calls. See `cloud-ui-validation.json`.
- All 11 native pointer scenarios passed again with the current 0.2.0 source. Real Win32 input exercised production WebView pointer handlers and native transfer commands with isolated synthetic accounts. Transfers, insertion order, reorder, detach/reattach, Escape, source-window closure, compose blocking, account ownership, folder, avatar, and color were preserved. See `native-drag-validation.json`.
- New native windows now fit the monitor work area. Three geometry tests passed, and the installed main window was visually checked at 225% scaling on a 2880×1800 display: Settings and the Cloud drives controls were reachable above the taskbar. This is a single-display result.
- Branding QA confirmed transparent PNG corners and all nine ICO sizes (16, 20, 24, 32, 40, 48, 64, 128, and 256 pixels). All nine new icon payloads are embedded in the executable. The Dubai wordmark uses no bundled font files.
- The executable and NSIS installer built as Yas Mail 0.2.0 with Windows GUI subsystem 2. All 29 runtime source inputs matched the build manifest. SHA-256 digests and the validation modes are recorded in `release-v020-validation.json`.
- The final installer updated the existing installation successfully (exit 0). The Start menu and desktop shortcuts target the verified installed executable. The installed app was opened and visually checked: the new logo appeared, Settings loaded cloud connections through native IPC, and the connection form opened without starting OAuth. No command-shell child was observed.

No live cloud-provider sign-in or end-to-end file workflow is claimed. Provider registrations and consented accounts are still required for those checks.

## Historical version 0.1.7 checks

The following checks were performed on the renamed 0.1.7 build. They are a historical baseline, not a fresh 0.2.0 result.

- JavaScript test suites: 31 tests passed after the rename. They cover contact import/export and health, writing checks, account avatars and selected-tab semantics, escaping/colors, keyboard navigation, saved tab order, transferred views, and insertion gaps. Demo-avatar assertions now expect YM.
- Frontend JavaScript syntax checks passed. Source scans found no old visible app name in the interface or branding assets. Remaining legacy storage keys are marked as compatibility identifiers.
- Rust formatting and all 17 Rust tests passed after the native rename.
- The Windows executable and NSIS installer built successfully. Product name, file description, and publisher are Yas Mail; the executable is `yas-mail.exe`, version 0.1.7, with Windows GUI subsystem 2.
- All 11 native pointer regression scenarios passed again on the renamed build with `yasmail:` events and Yas Mail window titles. Folder, avatar, color, ownership, insertion order, and drag cleanup were preserved. The compact report is `native-rename-validation.json`.
- The prior installation was removed without selecting app-data removal, and Yas Mail installed successfully in `YasMail-installed`. Windows Start lists Yas Mail and no longer lists the old name. The existing desktop shortcut was replaced with Yas Mail and points to the verified installed executable. The app launched through the new shortcut; its native window and interface showed Yas Mail, the YM demo avatar, and the new tagline.

## Historical functionality baseline

The following checks were performed on version 0.1.6 before the rename. They establish the functionality baseline, rather than claiming a fresh version 0.2.0 run.

- Rust suite: 17 tests passed, including 12 window tests for ownership, transfer, rollback, closure, geometry, DPI conversion, and emptied-source cleanup.
- Eleven native pointer scenarios passed through actual Windows SendInput, production WebView PointerEvents, and native transfer commands with isolated synthetic accounts at 225% scaling. They covered child-to-main, main-to-another-child and return at chosen gaps, reordering both ways, Escape, native drag-out, last-tab detachment/closure, reattachment, and compose blocking. Folder, avatar, color, exclusive ownership, and marker/ghost cleanup were preserved.
- A native crowded-receiver check passed with 19 existing tabs and an incoming account. Right-edge hovering scrolled 203.56 CSS pixels before insertion, the marker stayed within the visible strip, the source closed, and tab tops were not clipped.
- Browser checks covered mail reading/reply, calendar and contact creation, contact import/export and cleanup, preferences, signatures, writing checks, tone selection, account switching, tab order, transfer views, and compose-sender preservation. Provider/native window calls were mocked in those browser flows.
- A per-user NSIS install and missing-shortcut update repair completed successfully. The built and installed executables used Windows GUI subsystem 2 and launched without a command-shell child.
- A local Ollama rewrite request completed using `gemma3:1b`. Model quality still requires user review.

## Remaining limits

- Live Google/Microsoft authorization, mail delivery, calendar invitations, contact changes, or provider-side sender rules have not been verified against real accounts. They require OAuth registrations and consented test accounts.
- Live Google Drive, OneDrive, Dropbox, and Zoho WorkDrive sign-in, browsing, downloads, and existing-link workflows require the registrations described in CLOUD-DRIVES.md and consented test accounts. Live-provider end-to-end validation has not been established.
- Mixed-DPI/multiple-monitor dragging, native left-edge receiver scrolling, and manual human interaction feel remain unverified. Native input checks used one 225% display with synthetic accounts.
- Full native minimize/maximize/restore, header dragging, and double-click behavior have not been automated end to end.
- Windows Application Control previously blocked a release-profile Rust build script. Delivery uses the standard debug profile with the Windows GUI subsystem; no Windows security policy was changed.
- Native file-save dialogs and rewriting through the native compose UI were not fully automated. Browser behavior and the local model endpoint were checked separately.

This remains a preview with the feature boundaries described in README.md.
