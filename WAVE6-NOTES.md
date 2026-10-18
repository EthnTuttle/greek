# WAVE6-NOTES — Device Sync UI (Android, waves 6b/6c)

Android UI for device sync over the frozen wave-6 UniFFI contract. P1
implements the Rust exports; this wave ships the UI + seam stubs. Every stub
is marked `WAVE6-WIRE` in `RustCoreAdapter.kt` with the exact real call in a
comment — rewire at integration time.

## Used contract signatures (as frozen with P1)

```kotlin
// GreekCore gains:
fun syncDeviceId(): String
fun syncPairedDevices(): List<PairedDeviceView>     // deviceId, label, pairedAt, lastSyncHlc: String
fun syncExportBackupJson(): String
fun syncImportBackupJson(json: String): SyncStatsView
suspend fun syncNostrPush(): SyncStatsView           // throws GreekError.SyncException-mapped Kotlin exc
suspend fun syncNostrPull(): SyncStatsView
fun syncNostrConfig(): NostrConfigView               // relaysCsv: String, hasIdentity: Boolean
fun syncSetNostrRelays(relaysCsv: String)
fun syncGenerateNostrIdentity(): String              // npub for display
fun syncSetNostrNsec(nsec: String)                   // throws on invalid
suspend fun syncIrohTicket(): String                 // "greeksync://ticket/<...>"
suspend fun syncIrohJoin(ticket: String)
suspend fun syncIrohNow(): SyncStatsView
// SyncStatsView fields: reviewsSent: UInt, reviewsReceived: UInt, settingsApplied: UInt
```

## Seam (core/GreekCoreApi.kt) — 13 new members

`syncDeviceId`, `syncPairedDevices`, `syncExportBackupJson`,
`syncImportBackupJson` (plain) · `syncNostrPush`, `syncNostrPull`,
`syncIrohTicket`, `syncIrohJoin`, `syncIrohNow` (suspend) ·
`syncNostrConfig`, `syncSetNostrRelays`, `syncGenerateNostrIdentity`,
`syncSetNostrNsec` (plain)

Interface-level mirrors (generated views stay in `uniffi.uniffi_bindings`):

| Seam type | Generated view | Conversion at wire time |
|---|---|---|
| `SyncStats(sent, received, settings: Int)` | `SyncStatsView` | `reviewsSent/reviewsReceived/settingsApplied.toInt()` |
| `PairedDevice(deviceId, label, pairedAt, lastSyncHlc)` | `PairedDeviceView` | field-copy (all String) |
| `NostrConfig(relaysCsv, hasIdentity)` | `NostrConfigView` | field-copy |

## Stub behavior (RustCoreAdapter.kt, all `WAVE6-WIRE`)

| Member | Stub |
|---|---|
| `syncDeviceId` | `"unknown"` |
| `syncPairedDevices` | `emptyList()` |
| `syncExportBackupJson` | `"{}"` (valid JSON, zero content) |
| `syncImportBackupJson` | `SyncStats(0,0,0)` |
| `syncNostrPush/Pull` | `SyncStats(0,0,0)` |
| `syncNostrConfig` | `NostrConfig("", false)` |
| `syncSetNostrRelays` | no-op |
| `syncGenerateNostrIdentity` | `""` (UI treats blank as failure, shows error snackbar) |
| `syncSetNostrNsec` | no-op (real export must keep throwing on invalid keys) |
| `syncIrohTicket` | `""` (UI treats blank as failure) |
| `syncIrohJoin` | no-op |
| `syncIrohNow` | `SyncStats(0,0,0)` |

**Stubs compile against current bindings — no references to
`PairedDeviceView`/`SyncStatsView`/`NostrConfigView` anywhere yet.** Those
class names appear only inside `WAVE6-WIRE:` comments.

Integration guidance: fail-soft getters (`syncDeviceId`, `syncPairedDevices`,
`syncNostrConfig`, `syncExportBackupJson`) → reuse the
`runCatching + newApiAvailable` pattern from waves 2/3/5. Throwing exports
(`syncImportBackupJson`, push/pull, `syncSetNostrNsec`, `syncIrohJoin`) →
plain delegation; catch `UnsatisfiedLinkError` → `markApiUnavailable` +
rethrow as `UnsupportedOperationException` so the UI can surface real errors.

## UniFFI-vs-Kotlin quirks anticipated

- **UInt → Int**: `SyncStatsView` counts are u32 → `UInt`. The seam converts
  to `Int` immediately (`.toInt()`); the UI never feeds counts back to Rust,
  so the signedness loss is display-only and safe.
- **Async exports**: UniFFI 0.28 Kotlin maps async exports to `suspend fun` —
  the seam already declares them `suspend`, so wiring is body replacement
  only, no signature churn.
- **`GreekError.SyncException`**: generated Kotlin exceptions land as a
  mapped exception class on `.so` calls; both the VM (`busyCall` catch) and
  HomeViewModel (`runCatching`) treat them as generic `Throwable` → Log.w /
  error snackbar. No special handling needed, but don't swallow them silently
  in the adapter (see integration guidance above).
- **`lastSyncHlc` string shape unknown**: UI best-effort parses a leading
  digit run as epoch (s or ms) for relative time, falls back to showing the
  raw string, and blank → "Never synced". If P1 settles on a fixed epoch-ms
  format the parse stays correct.
- **`NostrConfigView` has no npub field**: "Identity: <npub first 16>…" is
  only displayed in the session that generated it (from
  `syncGenerateNostrIdentity()`'s return). A previously-configured identity
  shows as "Identity: configured". If P1 can add `npub` to the view cheaply,
  wire it into `SyncState.identityNpub` on refresh.

## DataStore additions (GreekApplication.kt)

- `sync_autolaunch` (bool, default true) — "Sync on app launch" toggle.
- `sync_iroh_joined` (bool, default false) — UI-side "iroh transport
  configured" marker set on a successful `syncIrohJoin`; the contract has no
  iroh-configuredness getter, so HomeViewModel's launch gate needs it.

## HomeViewModel launch auto-sync

`init { syncOnLaunch() }` — gated on toggle && (nostr hasIdentity || iroh
joined). Runs pull → iroh-now → push on Dispatchers.IO, silent (`Log.w` on
failure), refreshes home stats only when a round actually ran. Safe ordering:
`MainActivity` composes AppNavigation (and thus HomeViewModel) only after
`InitState.Ready`, so `CoreProvider.current()` is non-null by then.

## Dependency added

- `implementation("com.google.zxing:core:3.5.3")` — QR encoding of the iroh
  pairing ticket (BitMatrix → Bitmap in `SyncScreen.rememberQrBitmap`).

## Files touched

- `core/GreekCoreApi.kt` — 13 seam members + `SyncStats`/`PairedDevice`/`NostrConfig`
- `core/RustCoreAdapter.kt` — 13 WAVE6-WIRE stubs
- `viewmodel/SyncViewModel.kt` (new) — state + all sync actions
- `ui/screens/SyncScreen.kt` (new) — the "sync" destination
- `ui/screens/SettingsScreen.kt` — "Device sync (beta)" gear-row
- `navigation/AppNavigation.kt` — `Screen.Sync` route "sync" (pushed, no tab)
- `viewmodel/HomeViewModel.kt` — launch auto-sync
- `GreekApplication.kt` — 2 DataStore keys + SettingsRepository accessors
- `res/values/strings.xml` — 47 new strings (`sync_*`, `settings_sync_row_*`)
- `android/app/build.gradle.kts` — ZXing core dep

No `DataStoreSyncAdapter` existed — nothing else consumes `GreekCoreApi`, so
no seam default impls were needed.
