# Yas Mail account, cloud-drive, and AI setup

## Install and open Yas Mail 0.3.0

Run `Yas-Mail-Setup.exe` for a per-user installation, then search Start for **Yas Mail**. The installer does not require administrator access. It registers Yas Mail as an installed app and creates a Start menu shortcut to the installed executable. If a shortcut is missing during an update, the installer recreates it.

The portable alternative is `Yas-Mail-0.3.0.exe`. Running the portable file does not install the app or add a Start menu shortcut. Both variants use the Windows GUI subsystem, including the debug preview, so opening Yas Mail does not create a command window.

This machine uses `YasMail-installed` beside the delivery files as its installation location, and the real Windows Start shortcut points to that folder. Keep it in place. This explicit location avoids this coding environment's AppData redirection; running the installer yourself normally uses the standard per-user location.

For an unattended install, run `Yas-Mail-Setup.exe /S`. An unattended update/shortcut repair can use `Yas-Mail-Setup.exe /S /UPDATE`; omit `/NS` if you want shortcuts. Close Yas Mail before installing an update. Installing or repairing the shortcut does not connect an email account.

The Wing Letter logo and Windows icon use Yas Browser's royal-blue palette with a distinct envelope-and-wing mark. The wordmark uses installed Dubai Bold; the editable and rendered assets and nine-size ICO are documented in [branding/BRAND.md](branding/BRAND.md). No font files are bundled.

## Gmail, Outlook, and Microsoft 365

1. Select **＋** beside the account tabs.
2. Choose **Google** for Gmail/Google Workspace, or **Microsoft** for Outlook.com, Microsoft 365, or Exchange Online. The Microsoft option supports personal and work or school mailboxes through modern browser sign-in.
3. Your normal browser opens. Choose your account, sign in there, and review the requested access.
4. Return to Yas Mail when sign-in finishes. Your account appears in its own tab. Repeat to add another account.

You do not need an application registration, client ID, or client secret. Yas Mail's publisher supplies its Google and Microsoft application identities when building the app. Your email password is entered only on the provider's sign-in page. Work or school policies may require an administrator to approve Yas Mail.

The sign-in screen uses clear waiting/error states and subtle transitions. Animations follow the system's reduced-motion preference.

While Yas Mail is waiting for the browser callback, choose **Cancel sign-in** to stop waiting; closing the browser tab alone leaves the request waiting until it times out. Once the callback has been accepted, Yas Mail finishes processing the result.

### Sign-in is unavailable in this build

The current preview has no publisher Google/Microsoft credentials provisioned. It keeps unavailable provider buttons disabled and offers the demo rather than asking you for developer settings. The publisher must finish the one-time setup in [PUBLISHING.md](PUBLISHING.md) and distribute a configured build. No live provider sign-in has been established for this preview.

If a configured build later shows a provider error, try again and share the error with Yas Mail's publisher. `unauthorized_client` or "not enabled for consumers" is an application-registration issue for the publisher to resolve; users should not create their own registrations to work around it.

## Manual mail servers and on-premises Exchange

Select **＋ → Other email / manual setup** to connect another provider using **IMAP + SMTP**, **POP3 + SMTP**, or **SMTP only**. Enter the server details supplied by your mail provider or administrator, test the connection, then connect the account. Manual setup remains available without Yas Mail's Google/Microsoft publisher credentials.

Use **Microsoft** browser sign-in for a Microsoft 365 / Exchange Online mailbox. Manual username/password setup does not bypass Microsoft 365's authentication policies. For an on-premises Exchange server, ask your administrator for enabled IMAP or POP3 and SMTP services and their supported authentication. Yas Mail does not implement EWS, MAPI/HTTP, ActiveSync, or Exchange autodiscovery.

Manual accounts require TLS or STARTTLS with valid server certificates. POP3 leaves messages on the server; SMTP-only accounts have sending but no inbox. Contacts, calendars, meetings, and sender rules are not synchronized through these protocols. See [MANUAL-ACCOUNTS.md](MANUAL-ACCOUNTS.md) for ports, authentication, testing, and the exact feature limits.

## Cloud drives

Google Drive, OneDrive, Dropbox, and Zoho WorkDrive are read-only sources for email files. Each cloud connection uses its own browser OAuth sign-in. Follow [CLOUD-DRIVES.md](CLOUD-DRIVES.md) for provider details; mailbox sign-in alone does not connect a drive.

1. In the Windows app, open **Settings → Cloud drives → Connect cloud drive** and choose the provider. Google Drive and OneDrive use Yas Mail's publisher configuration; continue in your browser without entering application credentials. Dropbox and Zoho WorkDrive still require manual application details in this preview. You can also connect a drive from an open message's cloud picker.
2. Compose an email and choose **Attach from cloud**. Select a connected drive, browse its folders, or search for a file.
3. Choose **Attach copy** to fetch a downloadable file into the message. It shares the total attachment allowance with local files: 9 MB for Gmail or 3 MB for Outlook. Online files without a downloadable copy can be linked when an existing link is available.
4. Choose **Insert link** to add an existing provider link. Access stays as configured at the provider; recipients may need permission. Review the message and choose Send when ready.

The picker reads files and existing links without creating public links or changing sharing permissions. Connecting, browsing, or preparing a cloud file does not send mail. This first release focuses on mail file picking; background drive synchronization and upload management are not included. The browser demo does not connect real cloud accounts.

## Multiple accounts and signatures

Use the top-row **＋** button for each address. Each account appears as a Chrome-style tab with its avatar, email address, provider label, and color. Click a tab to switch, then open Settings to customize that account's avatar, color, signature, and style learning. New accounts receive stable, distinct default colors against Yas Mail's shared blue branding. Disconnect accounts in Settings; switching tabs does not disconnect them.

With a tab focused, use Left/Right Arrow, Home, or End to select another tab. In the native app, Ctrl+Tab moves to the next account and Ctrl+Shift+Tab moves to the previous one. Tabs scroll horizontally when needed. Each tab remembers its last section, folder, search, and message selection while the app stays open; these views are not saved to disk.

Drag an account tab left or right to change its position. Other tabs move in a live preview, and the strip scrolls automatically when you hold the dragged tab near an edge. Release within the tab row to save the order. Press Escape to cancel. Reordering does not change the active account. With a tab focused, Ctrl+Shift+Left Arrow or Ctrl+Shift+Right Arrow moves it one position.

To detach a connected account, drag its tab away from the tab row and release. You can also select it and use **Open account in new window** in the top toolbar. The new frameless window opens with that account's view and appearance. The account leaves the source window; it is not duplicated or disconnected. If that empties a detached source window, it closes. The main window stays open. Use **Return account to main window** in a detached window to move it back without signing in again.

To move a tab back by dragging, hold it over the main window's tab strip or the tab strip of another open Yas Mail window. Release at the blue insertion marker to place it in that gap. Its current folder, message, avatar, and color move with it. The account remains assigned to one window. If the detached source becomes empty, it closes; the main window stays open even when empty. Release away from a receiving tab strip to create a new window instead, or press Escape to cancel.

Receiving is blocked while the destination has an open modal, so moving a tab cannot change the sender behind a compose dialog. Cross-window transfers are supported within the same running Yas Mail app session; windows from separately launched app processes do not share the transfer registry. Version 0.1.7 previously passed eleven native pointer checks with synthetic accounts at 225% scaling. See [VALIDATION.md](VALIDATION.md) for the current release's results. Live-provider and mixed-DPI workflows remain unverified.

Each account is assigned to one open Yas Mail window at a time, and the native backend validates that ownership for account operations. The windows use the same saved normal account connections and preferences. If all accounts leave the main window, it shows an explicit empty state and disables compose rather than opening a demo account. You can connect another account there. A detached window closes when its last account moves elsewhere; closing a populated detached window returns its accounts.

Closing a window returns its accounts to the main window if it remains open, or to another remaining Yas Mail window. Closing the last window exits the app. All saved accounts start in the main window next time; the detached-window arrangement is not saved. Window detachment and transfer do not revoke provider access or remove saved credentials.

Yas Mail saves the ordered account IDs in local WebView storage and restores them on relaunch. This order applies only to this device. Disconnected accounts are ignored, and newly connected accounts append after the saved accounts. The saved order does not contain message or contact data.

The selected account is always the sender in compose. Switch accounts before composing; an open modal locks the account tabs while you write. Native window controls remain available at the top during a modal.

## Contact import, export, and cleanup

In Contacts, choose Import for UTF-8 CSV or vCard 3/4. CSV supports common Google/Outlook field headers; a simple template is `Name,Email,Phone`. Review new contacts before importing. Existing primary email matches are skipped. Imports create contacts at the selected provider and show partial failures. Import up to 500 contacts or 5 MB at once. Export saves the displayed name, primary email, and phone as CSV or vCard; it is not a full provider backup. CSV formula-like values are prefixed with an apostrophe for spreadsheet safety.

Contact health checks displayed primary fields. Matching emails/phones can indicate duplicates, but shared household or business numbers can also match. Compare before changing records. Suggested names are derived from email addresses and require your review. Missing numbers are never invented. Combine details updates one chosen contact and keeps the sources; it does not automatically merge or delete provider records. Contact deletion is a separate action with confirmation, and deletes the entire provider contact including fields not displayed in Yas Mail. Review full records at the provider before deletion.

## Local AI writing assistant setup

1. Install [Ollama for Windows](https://ollama.com/download/windows).
2. Run `ollama pull gemma3:1b` in a terminal. The model is a separate download of approximately 806 MB. You can select a larger local model if your hardware allows it.
3. Keep Ollama running on its default local address `127.0.0.1:11434`.
4. Open Yas Mail Settings, set the local model name, use **Check local runtime**, and save preferences.
5. Compose a message, choose **Rewrite with AI**, select the tone/action, and generate a preview. Review before applying or sending.

Yas Mail uses the local [Ollama generation API](https://docs.ollama.com/api/generate) and rejects cloud model entries. The app's local-AI HTTP client bypasses proxies and disallows redirects. It has no cloud fallback.

**Learn my writing style** is off by default and is set separately for each account. When enabled, a successfully submitted sent message contributes a short example (up to 300 characters). Up to five examples are retained in Windows Credential Manager. Quoted replies are excluded where recognized; examples may still contain personal information. Examples are included only when you request a personalized rewrite. Clear the memory in Settings at any time. Turning learning off stops collection; it does not delete existing examples. This feature does not fine-tune or retrain model weights.

Small models can change meaning or invent details despite instructions. Yas Mail previews all rewrites and gives the model no mail/calendar/contact tools. Browser/WebView spellchecking availability depends on installed Windows languages. The local basic writing checker is intentionally limited.
