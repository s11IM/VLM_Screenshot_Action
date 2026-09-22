# Security Policy

This is experimental desktop automation, not a security sandbox. Use a
disposable test account/desktop, keep sensitive windows closed, and supervise
real input. Do not run it with administrator privileges unless you understand
why the target requires them. Follow the target application's terms of use.

## Trust and Data Boundaries

- Screenshots, task text, and selected context are sent to the configured
  external API. The application does not perform local model inference.
- API keys are stored in WebView localStorage without application-level
  encryption. Chat text and screenshots are stored in IndexedDB.
- Local data is normally under `%LOCALAPPDATA%/com.vlmscreenshotaction.app/` on
  Windows. Cleaning build caches does not delete this data.
- Logs redact selected field names, not every possible secret or personal
  detail in arbitrary error strings. Review logs manually before sharing.
- Screen contents can contain prompt injections. Neither the fixed prompt nor
  tool argument validation guarantees that model decisions are safe.
- Mouse coordinates are bounded by the selected rectangle, but that rectangle
  is not locked to an application. Keyboard actions target a validated
  foreground window and can still invoke powerful OS/application shortcuts.
- F8 is checked during keyboard execution only. Use the UI stop control for
  other phases. Cancellation cannot undo input already sent.
- There is no total action/time/cost budget. Requests have a timeout and may
  retry once; the overall loop can continue until stopped or completed.
- The WebView CSP is currently disabled. Do not add remote scripts, remote UI
  navigation, or unsanitized HTML rendering without a security review.

## Reporting

Use this GitHub repository's **Security > Advisories > Report a vulnerability**
when private vulnerability reporting is enabled. Include affected versions,
minimal reproduction, impact, and a suggested fix if available. Remove all
credentials and unrelated personal data.

If private reporting is unavailable, open only a minimal issue asking the
maintainer to enable a private reporting channel. Do not publish exploit steps,
secrets, or sensitive screenshots in that issue. No private email address or
response-time guarantee is claimed by this policy.

If a credential has been exposed, revoke or rotate it first. Deleting a file
or changing a repository to private does not revoke an exposed credential.

## Maintenance Scope

Fixes are targeted at the current development branch. Historical tags do not
have a guaranteed backport schedule. Public packages must never contain
`user-data`, WebView profiles, logs, or exports from a real account.
See [the publishing checklist](docs/publishing.md).
