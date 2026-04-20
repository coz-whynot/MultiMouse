import { useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useStore } from './store/useStore';
import { TitleBar } from './components/TitleBar';
import { PinDisplay } from './components/PinModal';
import { TransferPanel } from './components/TransferProgress';
// UpdateBanner removed — update check now lives in Settings → About
import { BottomNav } from './components/BottomNav';
import { OnboardingWizard } from './components/OnboardingWizard';
import { CursorLandingOverlay } from './components/CursorLandingOverlay';
import { Home } from './pages/Home';
import { Layout } from './pages/Layout';
import { SettingsPage } from './pages/Settings';
import { PairingRequest, FileOffer, TransferInfo, IncomingDeepLink } from './types';

interface ReconnectState {
  peerId: string;
  peerName: string;
  attempt: number;
  maxAttempts: number;
  failed: boolean;
}

/** Compare two dotted semver strings ("0.2.0" vs "0.1.9"). Returns 1 / 0 / -1. */
export function cmpSemver(a: string, b: string): number {
  const pa = a.split('.').map((n) => parseInt(n, 10) || 0);
  const pb = b.split('.').map((n) => parseInt(n, 10) || 0);
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const x = pa[i] ?? 0;
    const y = pb[i] ?? 0;
    if (x !== y) return x > y ? 1 : -1;
  }
  return 0;
}

export default function App() {
  const {
    currentPage,
    setPage,
    peers,
    setPeers,
    setStatus,
    settings,
    setSettings,
    status,
    pairingRequest,
    setPairingRequest,
    setShownPin,
    setTransfers,
    setFileOffer,
    incomingDeepLink,
    setIncomingDeepLink,
    errorMsg,
    setErrorMsg,
    setAppVersion,
  } = useStore();


  const [draggingFiles, setDraggingFiles] = useState(false);
  const [reconnectState, setReconnectState] = useState<ReconnectState | null>(null);
  const [idleLockMsg, setIdleLockMsg] = useState<string | null>(null);
  // v0.3.12 — true when the platform denied rdev::grab at startup
  // (missing Accessibility / Input Monitoring on macOS, or the Windows
  // equivalent). Polled once on mount AND updated via the
  // `accessibility-needed` event for races where the event fires before
  // the UI mounts.
  const [inputGrabOk, setInputGrabOk] = useState<boolean>(true);
  const [permBusy, setPermBusy] = useState(false);
  // v0.3.13 — when true, the permissions banner expands to show a
  // platform-specific step-by-step for granting the permission and
  // restarting the app. Defaults open so users see the guide without
  // having to click first.
  const [permStepsOpen, setPermStepsOpen] = useState(true);
  // Detect platform on mount so the banner shows the right steps.
  // Falls back to macOS copy if the user agent doesn't expose platform
  // (some Tauri webview builds return empty).
  const [permPlatform, setPermPlatform] = useState<'macos' | 'windows' | 'linux'>(
    () => {
      const ua = navigator.userAgent || '';
      if (/Windows/i.test(ua)) return 'windows';
      if (/Linux/i.test(ua) && !/Android/i.test(ua)) return 'linux';
      return 'macos';
    }
  );
  // v0.3.8 Phase F — inbound log-pull request from the peer. When set, a
  // modal asks the local user "<requester> is requesting your diagnostic
  // logs; share?". Accept/reject both invoke the corresponding Rust command
  // to resolve the server-side oneshot, then this clears.
  const [logRequestFromPeer, setLogRequestFromPeer] = useState<{ requester_name: string } | null>(null);
  // Peer reported a newer app version → show a one-click update nudge.
  const [peerNewerVersion, setPeerNewerVersion] = useState<string | null>(null);
  const [updateNudgeBusy, setUpdateNudgeBusy] = useState(false);
  const reconnectDismissTimer = useRef<number | null>(null);
  const peersRef = useRef(peers);
  useEffect(() => {
    peersRef.current = peers;
  }, [peers]);

  // v0.3.12 — poll input_grab status once on mount so the banner
  // appears even when the UI mounts AFTER capture.rs has already
  // emitted its one-shot `accessibility-needed` event at startup.
  useEffect(() => {
    invoke<boolean>('get_input_grab_status')
      .then((ok) => setInputGrabOk(ok))
      .catch(() => {});
  }, []);
  // Tracks the most recent push-event timestamp so the 3s poll can avoid
  // overwriting event-driven state during narrow transition windows. When a
  // focus/relay/connection event fires and then the poll reads the backend
  // ~ms later, the backend's fresh value should win — but if the backend
  // command ran BEFORE the event (wrong-ordering from its perspective), the
  // poll would flicker the UI back. We gate the relay/connection/control
  // fields on a 500ms event-freshness window.
  const lastEventAt = useRef<number>(0);
  const markEvent = () => { lastEventAt.current = Date.now(); };

  const refresh = async () => {
    try {
      const [devices, stat, sett] = await Promise.all([
        invoke<any[]>('get_devices'),
        invoke<any>('get_status'),
        invoke<any>('get_settings'),
      ]);
      setPeers(devices);
      // If an event fired within the last 500ms, keep the event-driven
      // connection/relay/control fields that are already in the store and
      // only merge the rest of the status payload. Otherwise accept the
      // poll result wholesale.
      const sinceEvent = Date.now() - lastEventAt.current;
      if (sinceEvent < 500) {
        const existing = useStore.getState().status;
        if (existing) {
          setStatus({
            ...stat,
            relaying: existing.relaying,
            is_controlled: existing.is_controlled,
            connected_peer: existing.connected_peer,
          });
        } else {
          setStatus(stat);
        }
      } else {
        setStatus(stat);
      }
      setSettings(sett);
    } catch (e) {
      console.error('refresh error', e);
    }
  };

  useEffect(() => {
    refresh();
    // Load our own version once so DeviceCard / banners can compare against
    // peer-advertised versions without re-importing @tauri-apps/api/app.
    (async () => {
      try {
        const { getVersion } = await import('@tauri-apps/api/app');
        setAppVersion(await getVersion());
      } catch {}
    })();

    const lookupPeerName = (id: string): string => {
      const p = peersRef.current.find((x) => x.id === id);
      return p?.name ?? 'device';
    };

    const scheduleReconnectDismiss = (ms: number) => {
      if (reconnectDismissTimer.current != null) {
        window.clearTimeout(reconnectDismissTimer.current);
      }
      reconnectDismissTimer.current = window.setTimeout(() => {
        setReconnectState(null);
        reconnectDismissTimer.current = null;
      }, ms);
    };

    const unlisten = Promise.all([
      listen<string>('peer-version', async (e) => {
        // Compare to our own app version; nudge only if the peer is ahead.
        try {
          const { getVersion } = await import('@tauri-apps/api/app');
          const ours = await getVersion();
          const peer = (e.payload ?? '').trim();
          if (peer && cmpSemver(peer, ours) > 0) {
            setPeerNewerVersion(peer);
          }
        } catch {}
      }),
      listen('peers-updated', refresh),
      listen('connected', () => {
        markEvent();
        setShownPin(null);
        setReconnectState(null);
        if (reconnectDismissTimer.current != null) {
          window.clearTimeout(reconnectDismissTimer.current);
          reconnectDismissTimer.current = null;
        }
        refresh();
      }),
      listen('disconnected', () => {
        markEvent();
        setShownPin(null);
        // A pairing prompt still sitting open after disconnect is stale —
        // clear it so the next pairing doesn't surface the previous peer.
        setPairingRequest(null);
        // Optimistically clear event-driven fields so the UI updates instantly.
        const existing = useStore.getState().status;
        if (existing) {
          setStatus({
            ...existing,
            relaying: false,
            is_controlled: false,
            connected_peer: null,
          });
        }
        refresh();
      }),
      listen('relay-started', () => {
        markEvent();
        const existing = useStore.getState().status;
        if (existing) {
          setStatus({ ...existing, relaying: true });
        }
      }),
      listen<{ peer_id: string; attempt: number; max_attempts?: number }>(
        'reconnect-attempt',
        (e) => {
          const { peer_id, attempt, max_attempts } = e.payload;
          setReconnectState({
            peerId: peer_id,
            peerName: lookupPeerName(peer_id),
            attempt,
            maxAttempts: max_attempts ?? 5,
            failed: false,
          });
        },
      ),
      listen<string>('reconnect-gave-up', (e) => {
        const peerId = typeof e.payload === 'string' ? e.payload : String(e.payload ?? '');
        setReconnectState((prev) => ({
          peerId,
          peerName: prev?.peerName ?? lookupPeerName(peerId),
          attempt: prev?.attempt ?? 0,
          maxAttempts: prev?.maxAttempts ?? 5,
          failed: true,
        }));
        scheduleReconnectDismiss(4000);
      }),
      listen('focus-acquired', () => {
        markEvent();
        const existing = useStore.getState().status;
        if (existing) {
          setStatus({ ...existing, is_controlled: true });
        }
        refresh();
      }),
      listen('focus-released', () => {
        markEvent();
        const existing = useStore.getState().status;
        if (existing) {
          setStatus({ ...existing, is_controlled: false, relaying: false });
        }
        refresh();
      }),
      listen<PairingRequest>('pairing-request', (e) => setPairingRequest(e.payload)),
      listen<{ peer_id: string; pin: string }>('pin-shown', (e) => setShownPin(e.payload?.pin ?? null)),
      listen('pin-rejected', () => {
        setPairingRequest(null);
        setShownPin(null);
        setErrorMsg('Connection rejected or timed out.');
      }),
      listen<{ error: string }>('connection-failed', (e) => {
        setErrorMsg(e.payload?.error ?? 'Connection failed');
      }),
      listen<TransferInfo[]>('transfer-update', (e) => setTransfers(e.payload)),
      listen<FileOffer>('file-offer', (e) => setFileOffer(e.payload)),
      listen('transfer-complete', () => {}),
      listen<string>('deep-link', (e) => {
        const url = e.payload;
        try {
          const parsed = new URL(url);
          if (parsed.hostname === 'pair') {
            const code = parsed.searchParams.get('code');
            const host = parsed.searchParams.get('host');
            if (code && host) {
              setIncomingDeepLink({ code, host });
            }
          }
        } catch {
          // ignore malformed deep-link URLs
        }
      }),
      listen<boolean>('gaming-mode-changed', (e) => {
        const enabled = !!e.payload;
        const current = useStore.getState().settings;
        if (current) setSettings({ ...current, gaming_mode: enabled });
      }),
      listen('idle-lock-triggered', () => {
        const mins = useStore.getState().settings?.idle_lock_minutes ?? 0;
        setIdleLockMsg(
          mins > 0
            ? `Session locked after ${mins} minute${mins === 1 ? '' : 's'} of inactivity.`
            : 'Session locked due to inactivity.',
        );
        window.setTimeout(() => setIdleLockMsg(null), 6000);
      }),
      listen<{ requester_name: string }>('log-request-received', (e) => {
        setLogRequestFromPeer(e.payload);
      }),
      // v0.3.12 — capture.rs emits this on boot when rdev::grab fails.
      // Payload shape: { platform: "macos" | "windows" | "linux" }.
      listen<{ platform?: string }>('accessibility-needed', (e) => {
        setInputGrabOk(false);
        const p = e.payload?.platform;
        if (p === 'macos' || p === 'windows' || p === 'linux') setPermPlatform(p);
      }),
      listen('tauri://drag', () => setDraggingFiles(true)),
      listen('tauri://drag-leave', () => setDraggingFiles(false)),
      listen('tauri://drag-cancelled', () => setDraggingFiles(false)),
      listen('tauri://drag-drop', async (e: any) => {
        setDraggingFiles(false);
        const paths: string[] = e.payload?.paths ?? [];
        if (!paths.length) return;
        const store = useStore.getState();
        const connectedPeer = store.status?.connected_peer;
        if (!connectedPeer) return;
        try {
          await invoke('send_files', { peerId: connectedPeer, paths });
        } catch (err) {
          console.error('send_files error', err);
        }
      }),
    ]);

    const interval = setInterval(refresh, 3000);

    return () => {
      clearInterval(interval);
      if (reconnectDismissTimer.current != null) {
        window.clearTimeout(reconnectDismissTimer.current);
        reconnectDismissTimer.current = null;
      }
      unlisten.then((fns) => fns.forEach((fn) => fn()));
    };
  }, []);

  const connectedPeer = status?.connected_peer != null;
  const theme = settings?.theme === 'light' ? 'light' : 'dark';
  const isLight = theme === 'light';
  // Use the shared gradient token so App-level background matches the rest
  // of the UI whenever we introduce new themes later.
  const rootBackground = isLight
    ? 'linear-gradient(160deg, #f8fafc 0%, #eef2ff 45%, #e0e7ff 100%)'
    : 'linear-gradient(160deg, #0f0c29 0%, #1a0d4a 45%, #0d1535 100%)';
  const rootBorder = isLight ? '1px solid rgba(79,70,229,0.22)' : '1px solid rgba(99,102,241,0.16)';

  // FEATURE 2: Privacy blur while we're actively controlling a peer.
  // Default OFF — was blocking multi-monitor usage and hiding the Release
  // button. User can opt-in via Settings → Security.
  const privacyBlurOn = settings?.privacy_blur_on_relay === true;
  const relaying = status?.relaying === true;
  const isControlled = status?.is_controlled === true;
  const blurContent = privacyBlurOn && relaying;
  const peerNameForBlur = peersRef.current.find((p) => p.id === status?.connected_peer)?.name
    ?? status?.connected_peer
    ?? 'peer';

  // FEATURE 3: When we're being controlled, dim the root slightly so the user
  // can tell their machine is taking remote input.
  const dimForControlled = isControlled && !blurContent;

  const handleReleaseControl = () => invoke('release_cursor').catch(() => {});

  const handleAcceptDeepLink = async (link: IncomingDeepLink) => {
    try {
      await invoke('connect_to_device', { peerId: link.host, pin: link.code });
    } catch (e) {
      console.warn('deep-link connect failed', e);
    }
    setIncomingDeepLink(null);
    setPage('home');
  };

  return (
    <div
      data-theme={theme}
      className="w-full h-screen flex flex-col overflow-hidden relative"
      style={{
        background: rootBackground,
        borderRadius: '18px',
        border: rootBorder,
        boxShadow:
          theme === 'light'
            ? '0 32px 80px rgba(30,27,75,0.18), 0 0 0 1px rgba(255,255,255,0.5) inset, 0 1px 0 rgba(99,102,241,0.12) inset'
            : '0 32px 80px rgba(0,0,0,0.7), 0 0 0 1px rgba(255,255,255,0.04) inset, 0 1px 0 rgba(167,139,250,0.1) inset',
        opacity: dimForControlled ? 0.85 : 1,
        filter: dimForControlled ? 'saturate(0.8)' : 'none',
        transition: 'opacity 220ms ease, filter 220ms ease',
      }}
    >
      <TitleBar />

      {/* Peer-newer-version nudge — triggered by the PeerVersion protocol
          message. One-click "Update" runs the same tauri-plugin-updater
          flow as Settings → About's Check-for-update button. */}
      <AnimatePresence>
        {peerNewerVersion && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="mx-5 mt-1 overflow-hidden"
          >
            <div
              className="rounded-xl px-4 py-2.5 flex items-center gap-3 mb-1"
              style={{
                background: 'var(--accent-soft-bg)',
                border: '1px solid var(--accent-soft-br)',
              }}
            >
              <svg className="w-4 h-4 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="var(--accent-primary)" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0l-4 4m4-4v12" />
              </svg>
              <p className="text-[12px] flex-1 min-w-0" style={{ color: 'var(--text-secondary)' }}>
                Your peer is on v{peerNewerVersion}. Update this device to match.
              </p>
              <button
                disabled={updateNudgeBusy}
                onClick={async () => {
                  setUpdateNudgeBusy(true);
                  try {
                    const { check } = await import('@tauri-apps/plugin-updater');
                    const { relaunch } = await import('@tauri-apps/plugin-process');
                    const result = await check();
                    if (result?.available) {
                      await result.downloadAndInstall();
                      await relaunch();
                    } else {
                      // Peer reported newer, but our updater can't find a release yet
                      setErrorMsg('No release available yet — try again shortly.');
                      setPeerNewerVersion(null);
                    }
                  } catch (err) {
                    console.error('update failed', err);
                    setErrorMsg('Update failed — try again from Settings.');
                  } finally {
                    setUpdateNudgeBusy(false);
                  }
                }}
                className="text-[11px] font-bold rounded-lg px-2.5 py-1 transition-all active:scale-95 disabled:opacity-50 flex-shrink-0"
                style={{
                  background: 'var(--accent-primary)',
                  color: 'var(--text-inverse)',
                }}
              >
                {updateNudgeBusy ? 'Updating…' : 'Update'}
              </button>
              <button
                onClick={() => setPeerNewerVersion(null)}
                className="flex-shrink-0 transition-colors"
                style={{ color: 'var(--text-muted)' }}
                aria-label="Dismiss"
              >
                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Reconnect banner */}
      <AnimatePresence>
        {reconnectState && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="mx-5 mt-1 overflow-hidden"
          >
            <div
              className="rounded-xl px-3 py-2.5 flex items-center gap-2.5 mb-1"
              style={{
                background: reconnectState.failed
                  ? 'rgba(239,68,68,0.08)'
                  : 'rgba(245,158,11,0.08)',
                border: reconnectState.failed
                  ? '1px solid rgba(239,68,68,0.2)'
                  : '1px solid rgba(245,158,11,0.2)',
              }}
            >
              {reconnectState.failed ? (
                <svg className="w-3.5 h-3.5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="#f87171" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round"
                    d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                </svg>
              ) : (
                <motion.svg
                  className="w-3.5 h-3.5 flex-shrink-0"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="#fbbf24"
                  strokeWidth={2}
                  animate={{ rotate: 360 }}
                  transition={{ repeat: Infinity, duration: 1.2, ease: 'linear' }}
                >
                  <path strokeLinecap="round" strokeLinejoin="round"
                    d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                </motion.svg>
              )}
              <p
                className="text-[11px] flex-1 min-w-0"
                style={{ color: reconnectState.failed ? '#fca5a5' : '#fcd34d' }}
              >
                {reconnectState.failed
                  ? `Couldn't reconnect to ${reconnectState.peerName}. They may be offline.`
                  : `Reconnecting to ${reconnectState.peerName}… (attempt ${reconnectState.attempt} of ${reconnectState.maxAttempts})`}
              </p>
              <button
                onClick={() => {
                  setReconnectState(null);
                  if (reconnectDismissTimer.current != null) {
                    window.clearTimeout(reconnectDismissTimer.current);
                    reconnectDismissTimer.current = null;
                  }
                }}
                className="flex-shrink-0 w-5 h-5 rounded flex items-center justify-center transition-colors"
                style={{ color: reconnectState.failed ? 'rgba(248,113,113,0.5)' : 'rgba(251,191,36,0.6)' }}
              >
                <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Permissions banner (v0.3.12, expanded in v0.3.13). Shown when
          rdev::grab was denied at startup. Header row has the
          open-settings buttons; expandable body shows platform-specific
          step-by-step so the user doesn't have to guess the 5 manual
          clicks that come AFTER the pane opens. */}
      <AnimatePresence>
        {!inputGrabOk && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="mx-5 mt-1 overflow-hidden"
          >
            <div
              className="rounded-xl px-3 py-2.5 mb-1"
              style={{
                background: 'rgba(251,191,36,0.08)',
                border: '1px solid rgba(251,191,36,0.24)',
              }}
            >
              <div className="flex items-center gap-2.5 flex-wrap">
                <svg className="w-3.5 h-3.5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="#fbbf24" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round"
                    d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                </svg>
                <p className="text-[11px] flex-1 min-w-0" style={{ color: '#fcd34d' }}>
                  {permPlatform === 'windows'
                    ? "Input hook denied. MultiMouse can't read keys or drive the cursor until Windows grants input access, then restart the app."
                    : permPlatform === 'linux'
                    ? "Input capture isn't fully available on this Linux setup. MultiMouse falls back to listen-only — cross-device input will still work but cursor consume is disabled."
                    : "macOS Accessibility + Input Monitoring are needed. MultiMouse can't move the cursor or read keys until you grant both and restart the app."}
                </p>
                {permPlatform === 'macos' && (
                  <>
                    <button
                      disabled={permBusy}
                      onClick={async () => {
                        setPermBusy(true);
                        try { await invoke('open_input_permissions', { which: 'accessibility' }); }
                        catch {} finally { setPermBusy(false); }
                      }}
                      className="flex-shrink-0 px-2 py-1 rounded-lg text-[11px] font-semibold transition-all active:scale-95 disabled:opacity-60"
                      style={{ background: 'rgba(251,191,36,0.16)', border: '1px solid rgba(251,191,36,0.3)', color: '#fde68a' }}
                    >
                      Open Accessibility
                    </button>
                    <button
                      disabled={permBusy}
                      onClick={async () => {
                        setPermBusy(true);
                        try { await invoke('open_input_permissions', { which: 'input_monitoring' }); }
                        catch {} finally { setPermBusy(false); }
                      }}
                      className="flex-shrink-0 px-2 py-1 rounded-lg text-[11px] font-semibold transition-all active:scale-95 disabled:opacity-60"
                      style={{ background: 'rgba(251,191,36,0.16)', border: '1px solid rgba(251,191,36,0.3)', color: '#fde68a' }}
                    >
                      Open Input Monitoring
                    </button>
                  </>
                )}
                {permPlatform === 'windows' && (
                  <button
                    disabled={permBusy}
                    onClick={async () => {
                      setPermBusy(true);
                      try { await invoke('open_input_permissions', { which: 'accessibility' }); }
                      catch {} finally { setPermBusy(false); }
                    }}
                    className="flex-shrink-0 px-2 py-1 rounded-lg text-[11px] font-semibold transition-all active:scale-95 disabled:opacity-60"
                    style={{ background: 'rgba(251,191,36,0.16)', border: '1px solid rgba(251,191,36,0.3)', color: '#fde68a' }}
                  >
                    Open Privacy settings
                  </button>
                )}
                <button
                  onClick={() => setPermStepsOpen((v) => !v)}
                  className="flex-shrink-0 px-2 py-1 rounded-lg text-[11px] font-semibold transition-all"
                  style={{ color: '#fde68a', opacity: 0.8 }}
                >
                  {permStepsOpen ? 'Hide steps' : 'Show steps'}
                </button>
              </div>

              <AnimatePresence>
                {permStepsOpen && (
                  <motion.ol
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: 'auto' }}
                    exit={{ opacity: 0, height: 0 }}
                    className="mt-2 pl-5 text-[11px] leading-relaxed space-y-1"
                    style={{ color: '#fde68a', listStyle: 'decimal' }}
                  >
                    {permPlatform === 'macos' && (
                      <>
                        <li>Click <b>Open Accessibility</b> above → System Settings opens.</li>
                        <li>If MultiMouse is already listed, click <b>–</b> to remove it first (an older install leaves a stale entry).</li>
                        <li>Click the <b>+</b> button, pick <code>/Applications/MultiMouse.app</code>, and toggle it <b>on</b>.</li>
                        <li>Click <b>Open Input Monitoring</b> above and repeat the add/toggle for MultiMouse there too.</li>
                        <li>Quit MultiMouse from the <b>tray icon → Quit</b> (closing the window isn't enough — the process runs in the background).</li>
                        <li>Reopen MultiMouse from Applications. The banner should disappear.</li>
                      </>
                    )}
                    {permPlatform === 'windows' && (
                      <>
                        <li>Click <b>Open Privacy settings</b> above.</li>
                        <li>Look for input / keyboard / mouse access sections and make sure apps are allowed to access these.</li>
                        <li>If Windows Defender / an anti-virus prompted about MultiMouse's input hook on first launch and you clicked Block, you'll need to allow it via the Defender history.</li>
                        <li>Right-click the MultiMouse tray icon → <b>Quit</b>.</li>
                        <li>Reopen MultiMouse. The banner should disappear.</li>
                      </>
                    )}
                    {permPlatform === 'linux' && (
                      <>
                        <li>No action required — the app is running in listen-only mode.</li>
                        <li>Input relay still works across devices; you just won't be able to consume keys on the receiving side.</li>
                        <li>For full grab on Linux you'd need a distro / compositor with the rdev <code>unstable_grab</code> path enabled, which isn't shipped in this build.</li>
                      </>
                    )}
                  </motion.ol>
                )}
              </AnimatePresence>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Error banner */}
      <AnimatePresence>
        {errorMsg && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="mx-5 mt-1 overflow-hidden"
          >
            <div
              className="rounded-xl px-3 py-2.5 flex items-center gap-2.5 mb-1"
              style={{
                background: 'rgba(239,68,68,0.08)',
                border: '1px solid rgba(239,68,68,0.2)',
              }}
            >
              <svg className="w-3.5 h-3.5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="#f87171" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round"
                  d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
              </svg>
              <p className="text-[11px] flex-1 min-w-0" style={{ color: '#fca5a5' }}>{errorMsg}</p>
              <button
                onClick={() => setErrorMsg(null)}
                className="flex-shrink-0 w-5 h-5 rounded flex items-center justify-center transition-colors"
                style={{ color: 'rgba(248,113,113,0.5)' }}
              >
                <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <TransferPanel />

      {/* File drag overlay. Shown ONLY when a peer is connected — without
          a peer there's no send target and a "No device connected" overlay
          is user-hostile when the user's just dragging a file between two
          windows that happens to pass over this one. Text names the peer
          so the user sees which device will receive the file. */}
      <AnimatePresence>
        {draggingFiles && connectedPeer && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="absolute inset-0 z-40 flex items-center justify-center pointer-events-none"
            style={{
              borderRadius: '18px',
              background: 'rgba(99,102,241,0.14)',
              backdropFilter: 'blur(3px)',
              border: '2px dashed rgba(99,102,241,0.55)',
            }}
          >
            <div className="flex flex-col items-center gap-3">
              <motion.div
                animate={{ y: [0, -8, 0] }}
                transition={{ repeat: Infinity, duration: 1.3, ease: 'easeInOut' }}
              >
                <svg className="w-14 h-14" fill="none" viewBox="0 0 24 24" stroke={isLight ? '#4f46e5' : 'rgba(167,139,250,0.85)'} strokeWidth={1.3}>
                  <path strokeLinecap="round" strokeLinejoin="round"
                    d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" />
                </svg>
              </motion.div>
              <div className="text-center">
                <p className="text-sm font-bold" style={{ color: 'var(--text-strong)' }}>
                  Drop to send to {peerNameForBlur}
                </p>
                <p className="text-xs mt-0.5" style={{ color: 'var(--text-muted)' }}>
                  File will land in their Downloads folder
                </p>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Page content (blurred when actively controlling a peer) */}
      <div className="flex flex-col flex-1 overflow-hidden relative">
        <div
          className="flex flex-col flex-1 overflow-hidden"
          style={{
            filter: blurContent ? 'blur(12px)' : 'none',
            transition: 'filter 220ms ease',
          }}
          aria-hidden={blurContent || undefined}
        >
          <AnimatePresence mode="wait">
            {currentPage === 'home' && (
              <motion.div
                key="home"
                initial={{ opacity: 0, x: -12 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: 12 }}
                transition={{ duration: 0.18, ease: 'easeOut' }}
                className="flex flex-col flex-1 overflow-hidden"
              >
                <Home />
              </motion.div>
            )}
            {currentPage === 'layout' && (
              <motion.div
                key="layout"
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -10 }}
                transition={{ duration: 0.18, ease: 'easeOut' }}
                className="flex flex-col flex-1 overflow-hidden"
              >
                <Layout />
              </motion.div>
            )}
            {currentPage === 'settings' && (
              <motion.div
                key="settings"
                initial={{ opacity: 0, x: 12 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: -12 }}
                transition={{ duration: 0.18, ease: 'easeOut' }}
                className="flex flex-col flex-1 overflow-hidden"
              >
                <SettingsPage />
              </motion.div>
            )}
          </AnimatePresence>
        </div>

        {/* Privacy overlay — mounted ABOVE the blurred content, but lets the
            "Release" button remain reachable for emergency bail-out. */}
        <AnimatePresence>
          {blurContent && (
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="absolute inset-0 flex flex-col items-center justify-center px-6 z-30"
              style={{ background: isLight ? 'rgba(30,27,75,0.18)' : 'rgba(6,6,16,0.35)' }}
            >
              <div
                className="rounded-3xl px-5 py-4 flex flex-col items-center gap-3 max-w-[82%]"
                style={{
                  background: isLight
                    ? 'linear-gradient(160deg, rgba(255,255,255,0.97) 0%, rgba(238,242,255,0.95) 100%)'
                    : 'linear-gradient(160deg, rgba(22,20,50,0.92) 0%, rgba(16,14,38,0.92) 100%)',
                  border: `1px solid ${isLight ? 'rgba(79,70,229,0.32)' : 'rgba(99,102,241,0.3)'}`,
                  boxShadow: isLight
                    ? '0 18px 48px rgba(30,27,75,0.28)'
                    : '0 18px 48px rgba(0,0,0,0.45)',
                }}
              >
                <p className="text-center text-sm font-semibold" style={{ color: 'var(--text-strong)' }}>
                  🔒 Controlling {peerNameForBlur} — content hidden for privacy
                </p>
                <p className="text-center text-[11px]" style={{ color: 'var(--text-muted)' }}>
                  Press ESC ESC, or use the button below, to return cursor to this machine.
                </p>
                <button
                  onClick={handleReleaseControl}
                  className="text-xs px-3 py-1.5 rounded-xl font-semibold transition-all active:scale-95"
                  style={{
                    background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                    color: 'white',
                    boxShadow: '0 4px 14px rgba(99,102,241,0.35)',
                  }}
                >
                  Release control
                </button>
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      <BottomNav current={currentPage} onChange={setPage} connectedPeer={connectedPeer} />

      {/* Pairing modal */}
      <AnimatePresence>
        {pairingRequest && (
          <PinDisplay
            peerName={pairingRequest.peer_name}
            pin={pairingRequest.pin}
            onClose={() => {
              setPairingRequest(null);
              // Also clear the client-side SAS or Home.tsx's connecting overlay
              // will keep showing an old PIN into the next pairing attempt.
              setShownPin(null);
            }}
          />
        )}
      </AnimatePresence>

      {/* Inbound log-pull request modal (v0.3.8 Phase F) */}
      <AnimatePresence>
        {logRequestFromPeer && (
          <LogRequestModal
            requesterName={logRequestFromPeer.requester_name}
            onAccept={async () => {
              try { await invoke('accept_log_request'); } catch {}
              setLogRequestFromPeer(null);
            }}
            onReject={async () => {
              try { await invoke('reject_log_request'); } catch {}
              setLogRequestFromPeer(null);
            }}
          />
        )}
      </AnimatePresence>

      {/* First-run onboarding */}
      <AnimatePresence>
        {settings && settings.onboarding_done === false && <OnboardingWizard />}
      </AnimatePresence>

      {/* Cursor landing overlay — only while being controlled */}
      {isControlled && <CursorLandingOverlay />}

      {/* Idle auto-lock toast */}
      <AnimatePresence>
        {idleLockMsg && (
          <motion.div
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 12 }}
            className="absolute bottom-20 left-1/2 -translate-x-1/2 z-50"
          >
            <div
              className="rounded-2xl px-4 py-2.5 flex items-center gap-2.5"
              style={{
                background: isLight ? 'rgba(255,255,255,0.96)' : 'rgba(22,20,50,0.96)',
                border: `1px solid ${isLight ? 'rgba(79,70,229,0.3)' : 'rgba(99,102,241,0.3)'}`,
                boxShadow: isLight
                  ? '0 18px 48px rgba(30,27,75,0.22)'
                  : '0 18px 48px rgba(0,0,0,0.45)',
              }}
            >
              <svg className="w-4 h-4 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="var(--accent-primary)" strokeWidth={1.8}>
                <path strokeLinecap="round" strokeLinejoin="round"
                  d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
              </svg>
              <p className="text-xs" style={{ color: 'var(--text-strong)' }}>{idleLockMsg}</p>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Deep-link pairing modal */}
      <AnimatePresence>
        {incomingDeepLink && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 flex items-center justify-center z-50"
            style={{ background: isLight ? 'rgba(30,27,75,0.5)' : 'rgba(6,6,16,0.88)', backdropFilter: 'blur(16px)' }}
          >
            <motion.div
              initial={{ scale: 0.88, opacity: 0, y: 12 }}
              animate={{ scale: 1, opacity: 1, y: 0 }}
              exit={{ scale: 0.88, opacity: 0, y: 12 }}
              transition={{ type: 'spring', stiffness: 420, damping: 32 }}
              className="w-[280px] rounded-3xl p-6"
              style={{
                background: isLight
                  ? 'linear-gradient(160deg, rgba(255,255,255,0.98) 0%, rgba(238,242,255,0.96) 100%)'
                  : 'linear-gradient(160deg, rgba(22,20,50,0.98) 0%, rgba(16,14,38,0.98) 100%)',
                border: `1px solid ${isLight ? 'rgba(79,70,229,0.22)' : 'rgba(99,102,241,0.2)'}`,
                boxShadow: isLight
                  ? '0 24px 64px rgba(30,27,75,0.24), 0 0 0 1px rgba(255,255,255,0.6) inset'
                  : '0 24px 64px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.04) inset',
              }}
            >
              <div className="flex flex-col items-center mb-4">
                <div
                  className="w-12 h-12 rounded-2xl flex items-center justify-center mb-3"
                  style={{
                    background: 'linear-gradient(135deg, rgba(99,102,241,0.25), rgba(168,85,247,0.18))',
                    border: '1px solid rgba(99,102,241,0.3)',
                  }}
                >
                  <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke={isLight ? '#4f46e5' : 'rgba(167,139,250,0.9)'} strokeWidth={1.8}>
                    <path strokeLinecap="round" strokeLinejoin="round"
                      d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
                  </svg>
                </div>
                <p
                  className="text-[11px] font-semibold uppercase tracking-widest mb-1"
                  style={{ color: 'var(--accent-muted)' }}
                >
                  Pairing Link
                </p>
                <p className="font-bold text-base leading-tight text-center" style={{ color: 'var(--text-primary)' }}>
                  Accept pairing request?
                </p>
              </div>

              <p className="text-center text-[13px] mb-3 leading-relaxed" style={{ color: 'var(--text-muted)' }}>
                Connect to{' '}
                <span className="font-semibold font-mono" style={{ color: 'var(--text-strong)' }}>{incomingDeepLink.host}</span>
                ?
              </p>

              <div
                className="mb-4 rounded-2xl py-3 px-4"
                style={{
                  background: 'var(--accent-soft-bg)',
                  border: '1px solid var(--accent-soft-br)',
                }}
              >
                <p
                  className="text-center text-[10px] font-semibold uppercase tracking-widest mb-1.5"
                  style={{ color: 'var(--accent-muted)' }}
                >
                  Pairing code
                </p>
                <p
                  className="text-center font-mono font-bold tracking-[0.3em] text-2xl"
                  style={{
                    color: 'var(--text-primary)',
                    textShadow: isLight ? 'none' : '0 2px 8px rgba(99,102,241,0.4)',
                  }}
                >
                  {incomingDeepLink.code}
                </p>
              </div>

              <div className="flex gap-2">
                <button
                  onClick={() => setIncomingDeepLink(null)}
                  className="flex-1 py-2.5 rounded-xl text-sm font-medium transition-all active:scale-[0.97]"
                  style={{
                    background: 'var(--bg-subtle)',
                    border: '1px solid var(--border-subtle)',
                    color: 'var(--text-body)',
                  }}
                >
                  Reject
                </button>
                <button
                  onClick={() => handleAcceptDeepLink(incomingDeepLink)}
                  className="flex-1 py-2.5 rounded-xl text-sm font-bold transition-all active:scale-[0.97]"
                  style={{
                    background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                    boxShadow: '0 4px 16px rgba(99,102,241,0.35)',
                    color: 'white',
                  }}
                >
                  Accept
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/* v0.3.8 Phase F — modal shown when a peer requests our diagnostic logs.
   Must be explicit accept: a silent auto-share would let a malicious or
   compromised peer exfiltrate logs with no user signal. Rejecting the
   modal (or ignoring it for 60s) sends back an empty `LogShare`, so the
   requester's UI unblocks with a "peer declined" message. */
function LogRequestModal({
  requesterName,
  onAccept,
  onReject,
}: {
  requesterName: string;
  onAccept: () => void;
  onReject: () => void;
}) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-50 flex items-center justify-center p-6"
      style={{ background: 'rgba(0,0,0,0.55)' }}
    >
      <motion.div
        initial={{ scale: 0.95, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        exit={{ scale: 0.95, opacity: 0 }}
        className="w-full max-w-sm rounded-2xl p-5"
        style={{
          background: 'var(--bg-card)',
          border: '1px solid var(--border-strong)',
          boxShadow: '0 16px 48px rgba(0,0,0,0.45)',
        }}
      >
        <p
          className="text-[10px] font-bold uppercase tracking-widest mb-2"
          style={{ color: 'var(--accent-muted)' }}
        >
          Log request
        </p>
        <p
          className="text-base font-semibold mb-1.5 break-words"
          style={{ color: 'var(--text-primary)' }}
        >
          {requesterName || 'Peer'} is asking for your diagnostic logs.
        </p>
        <p
          className="text-xs leading-relaxed mb-4"
          style={{ color: 'var(--text-muted)' }}
        >
          Accept to share the last ~500 lines of your log. This contains
          connection events and warnings, not your keyboard activity or
          clipboard content. Reject if you're unsure.
        </p>
        <div className="flex gap-2">
          <button
            onClick={onReject}
            className="flex-1 py-2.5 rounded-xl text-sm font-medium transition-all active:scale-[0.97]"
            style={{
              background: 'var(--bg-subtle)',
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-body)',
            }}
          >
            Reject
          </button>
          <button
            onClick={onAccept}
            className="flex-1 py-2.5 rounded-xl text-sm font-bold transition-all active:scale-[0.97]"
            style={{
              background: 'linear-gradient(135deg, #6366f1, #a855f7)',
              boxShadow: '0 4px 16px rgba(99,102,241,0.35)',
              color: 'white',
            }}
          >
            Share logs
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
}
