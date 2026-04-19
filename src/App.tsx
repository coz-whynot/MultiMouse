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
  } = useStore();

  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [draggingFiles, setDraggingFiles] = useState(false);
  const [reconnectState, setReconnectState] = useState<ReconnectState | null>(null);
  const [idleLockMsg, setIdleLockMsg] = useState<string | null>(null);
  const reconnectDismissTimer = useRef<number | null>(null);
  const peersRef = useRef(peers);
  peersRef.current = peers;

  const refresh = async () => {
    try {
      const [devices, stat, sett] = await Promise.all([
        invoke<any[]>('get_devices'),
        invoke<any>('get_status'),
        invoke<any>('get_settings'),
      ]);
      setPeers(devices);
      setStatus(stat);
      setSettings(sett);
    } catch (e) {
      console.error('refresh error', e);
    }
  };

  useEffect(() => {
    refresh();

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
      listen('peers-updated', refresh),
      listen('connected', () => {
        setShownPin(null);
        setReconnectState(null);
        if (reconnectDismissTimer.current != null) {
          window.clearTimeout(reconnectDismissTimer.current);
          reconnectDismissTimer.current = null;
        }
        refresh();
      }),
      listen('disconnected', () => { setShownPin(null); refresh(); }),
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
      listen('focus-acquired', refresh),
      listen('focus-released', refresh),
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
      listen('idle-lock-triggered', () => {
        const mins = useStore.getState().settings?.idle_lock_minutes ?? 0;
        setIdleLockMsg(
          mins > 0
            ? `Session locked after ${mins} minute${mins === 1 ? '' : 's'} of inactivity.`
            : 'Session locked due to inactivity.',
        );
        window.setTimeout(() => setIdleLockMsg(null), 6000);
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
  const rootBackground =
    theme === 'light'
      ? 'linear-gradient(160deg, #f8fafc 0%, #ede9fe 45%, #e0e7ff 100%)'
      : 'linear-gradient(160deg, #0f0c29 0%, #1a0d4a 45%, #0d1535 100%)';
  const rootBorder = theme === 'light' ? '1px solid rgba(99,102,241,0.22)' : '1px solid rgba(99,102,241,0.16)';

  // FEATURE 2: Privacy blur while we're actively controlling a peer.
  // TODO: expose a `privacy_blur_on_relay` setting once the backend adds it.
  const privacyBlurOn = true;
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
    // Best-effort: forward the pairing host + code to the backend. Exact
    // command name is up to the backend agent; we try a couple of shapes and
    // always clear the modal so the UI doesn't get stuck.
    try {
      await invoke('connect_to_host', { host: link.host, code: link.code });
    } catch {
      try {
        await invoke('connect_to_device', { peerId: link.host, pin: link.code });
      } catch (e) {
        console.warn('deep-link connect failed', e);
      }
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

      {/* Content column — stays at a readable width even when the window is wide,
          so cards don't stretch edge-to-edge and feel cramped. */}
      <div className="flex flex-col flex-1 overflow-hidden mx-auto w-full max-w-[520px] px-1">

      {/* Reconnect banner */}
      <AnimatePresence>
        {reconnectState && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="mx-3 mt-1 overflow-hidden"
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

      {/* Error banner */}
      <AnimatePresence>
        {errorMsg && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="mx-3 mt-1 overflow-hidden"
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

      {/* File drag overlay */}
      <AnimatePresence>
        {draggingFiles && (
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
                <svg className="w-14 h-14" fill="none" viewBox="0 0 24 24" stroke="rgba(167,139,250,0.85)" strokeWidth={1.3}>
                  <path strokeLinecap="round" strokeLinejoin="round"
                    d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" />
                </svg>
              </motion.div>
              <div className="text-center">
                <p className="text-sm font-bold" style={{ color: 'rgba(255,255,255,0.85)' }}>
                  {connectedPeer ? 'Drop to send' : 'No device connected'}
                </p>
                {connectedPeer && (
                  <p className="text-xs mt-0.5" style={{ color: 'rgba(255,255,255,0.4)' }}>
                    File will land in their Downloads folder
                  </p>
                )}
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
              style={{ background: 'rgba(6,6,16,0.35)' }}
            >
              <div
                className="rounded-3xl px-5 py-4 flex flex-col items-center gap-3 max-w-[82%]"
                style={{
                  background: 'linear-gradient(160deg, rgba(22,20,50,0.92) 0%, rgba(16,14,38,0.92) 100%)',
                  border: '1px solid rgba(99,102,241,0.3)',
                  boxShadow: '0 18px 48px rgba(0,0,0,0.45)',
                }}
              >
                <p className="text-center text-sm font-semibold" style={{ color: 'rgba(255,255,255,0.92)' }}>
                  🔒 Controlling {peerNameForBlur} — content hidden for privacy
                </p>
                <p className="text-center text-[11px]" style={{ color: 'rgba(255,255,255,0.48)' }}>
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

      <BottomNav current={currentPage as any} onChange={setPage as any} connectedPeer={connectedPeer} />

      </div> {/* end centered content column */}

      {/* Pairing modal */}
      <AnimatePresence>
        {pairingRequest && (
          <PinDisplay
            peerName={pairingRequest.peer_name}
            pin={pairingRequest.pin}
            onClose={() => setPairingRequest(null)}
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
                background: 'rgba(22,20,50,0.96)',
                border: '1px solid rgba(99,102,241,0.3)',
                boxShadow: '0 18px 48px rgba(0,0,0,0.45)',
              }}
            >
              <svg className="w-4 h-4 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="#a78bfa" strokeWidth={1.8}>
                <path strokeLinecap="round" strokeLinejoin="round"
                  d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
              </svg>
              <p className="text-xs" style={{ color: 'rgba(255,255,255,0.85)' }}>{idleLockMsg}</p>
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
            style={{ background: 'rgba(6,6,16,0.88)', backdropFilter: 'blur(16px)' }}
          >
            <motion.div
              initial={{ scale: 0.88, opacity: 0, y: 12 }}
              animate={{ scale: 1, opacity: 1, y: 0 }}
              exit={{ scale: 0.88, opacity: 0, y: 12 }}
              transition={{ type: 'spring', stiffness: 420, damping: 32 }}
              className="w-[280px] rounded-3xl p-6"
              style={{
                background: 'linear-gradient(160deg, rgba(22,20,50,0.98) 0%, rgba(16,14,38,0.98) 100%)',
                border: '1px solid rgba(99,102,241,0.2)',
                boxShadow: '0 24px 64px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.04) inset',
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
                  <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="rgba(167,139,250,0.9)" strokeWidth={1.8}>
                    <path strokeLinecap="round" strokeLinejoin="round"
                      d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
                  </svg>
                </div>
                <p
                  className="text-[11px] font-semibold uppercase tracking-widest mb-1"
                  style={{ color: 'rgba(167,139,250,0.55)' }}
                >
                  Pairing Link
                </p>
                <p className="text-white font-bold text-base leading-tight text-center">
                  Accept pairing request?
                </p>
              </div>

              <p className="text-center text-[13px] mb-3 leading-relaxed" style={{ color: 'rgba(255,255,255,0.45)' }}>
                Connect to{' '}
                <span className="text-white/80 font-semibold font-mono">{incomingDeepLink.host}</span>
                ?
              </p>

              <div
                className="mb-4 rounded-2xl py-3 px-4"
                style={{
                  background: 'rgba(99,102,241,0.08)',
                  border: '1px solid rgba(99,102,241,0.22)',
                }}
              >
                <p
                  className="text-center text-[10px] font-semibold uppercase tracking-widest mb-1.5"
                  style={{ color: 'rgba(167,139,250,0.65)' }}
                >
                  Pairing code
                </p>
                <p
                  className="text-center text-white font-mono font-bold tracking-[0.3em] text-2xl"
                  style={{ textShadow: '0 2px 8px rgba(99,102,241,0.4)' }}
                >
                  {incomingDeepLink.code}
                </p>
              </div>

              <div className="flex gap-2">
                <button
                  onClick={() => setIncomingDeepLink(null)}
                  className="flex-1 py-2.5 rounded-xl text-sm font-medium transition-all active:scale-[0.97]"
                  style={{
                    background: 'rgba(255,255,255,0.05)',
                    border: '1px solid rgba(255,255,255,0.1)',
                    color: 'rgba(255,255,255,0.55)',
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
