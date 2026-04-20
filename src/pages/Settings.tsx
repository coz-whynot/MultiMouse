import { useState, useEffect, useMemo, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { useStore } from '../store/useStore';
import { Settings, KnownDevice, BandwidthStats, AuditEntry } from '../types';

/* ── Byte / time formatters for Session Stats ── */
const formatBytes = (n: number): string => {
  if (!Number.isFinite(n) || n <= 0) return '0 B';
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
};

const formatUptime = (secs: number): string => {
  if (!Number.isFinite(secs) || secs <= 0) return '0s';
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = Math.floor(secs % 60);
  if (h > 0) return `${h}h ${m}m ${s}s`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
};

/* ── Relative time for audit log ── */
const formatRelativeTime = (unixSecs: number): string => {
  const now = Date.now() / 1000;
  const delta = Math.max(0, now - unixSecs);
  if (delta < 60) return 'just now';
  if (delta < 3600) {
    const m = Math.floor(delta / 60);
    return `${m} min ago`;
  }
  if (delta < 86400) {
    const h = Math.floor(delta / 3600);
    return h === 1 ? '1 hour ago' : `${h} hours ago`;
  }
  if (delta < 172800) return 'yesterday';
  if (delta < 86400 * 7) {
    const d = Math.floor(delta / 86400);
    return `${d} days ago`;
  }
  const dt = new Date(unixSecs * 1000);
  return dt.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
};

/* ── Color dot per audit action ── */
const auditDotColor = (action: string): string => {
  if (action === 'connected') return 'var(--success)';
  if (action === 'pairing_rejected') return 'var(--danger)';
  return 'var(--text-faint)';
};

const auditDotEmoji = (action: string): string => {
  if (action === 'connected') return '🟢';
  if (action === 'pairing_rejected') return '🔴';
  return '⚫';
};

const auditLabel = (action: string): string => {
  if (action === 'connected') return 'connected';
  if (action === 'disconnected') return 'disconnected';
  if (action === 'pairing_rejected') return 'pairing rejected';
  return action.replace(/_/g, ' ');
};

/* ── Toggle switch ── */
const Toggle = ({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) => (
  <div className="flex items-center justify-between gap-4">
    <div className="flex-1 min-w-0">
      <p className="text-sm font-medium" style={{ color: 'var(--text-secondary)' }}>{label}</p>
      {description && (
        <p className="text-[11px] mt-0.5 leading-relaxed" style={{ color: 'var(--text-faint)' }}>
          {description}
        </p>
      )}
    </div>
    <button
      onClick={() => onChange(!checked)}
      aria-checked={checked}
      role="switch"
      className="relative rounded-full flex-shrink-0 transition-colors duration-200 focus:outline-none"
      style={{
        height: 24,
        width: 44,
        background: checked
          ? 'linear-gradient(90deg, #6366f1, #a855f7)'
          : 'var(--bg-hover)',
        boxShadow: checked ? '0 2px 8px rgba(99,102,241,0.35)' : 'none',
      }}
    >
      <motion.span
        animate={{ x: checked ? 20 : 2 }}
        transition={{ type: 'spring', stiffness: 600, damping: 32 }}
        className="absolute top-[3px] w-[18px] h-[18px] rounded-full shadow-md block"
        style={{ background: 'white' }}
      />
    </button>
  </div>
);

/* ── Section container ── */
const Section = ({
  title,
  icon,
  children,
  className,
}: {
  title: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) => (
  <div className={`space-y-2 ${className ?? ''}`}>
    <div className="flex items-center gap-2 px-1">
      {icon && <span style={{ color: 'var(--text-muted)' }}>{icon}</span>}
      <p
        className="text-[10px] font-bold uppercase tracking-widest"
        style={{ color: 'var(--text-muted)' }}
      >
        {title}
      </p>
    </div>
    <div
      className="rounded-2xl overflow-hidden"
      style={{
        background: 'var(--bg-subtle-2)',
        border: '1px solid var(--border-subtle)',
      }}
    >
      {children}
    </div>
  </div>
);

/* ── Row inside section ── */
const Row = ({ children, noDivider }: { children: React.ReactNode; noDivider?: boolean }) => (
  <div
    className="px-4 py-3.5"
    style={{
      borderBottom: noDivider ? 'none' : '1px solid var(--divider)',
    }}
  >
    {children}
  </div>
);

/* ── Developer tools section (v0.3.9) ──
   Live runtime-state viewer + three action buttons for users hitting
   "edge-cross doesn't fire" or "peer reconnect blocked". Polls
   `get_debug_state` at 1 Hz while connected so users can WATCH the flags
   flip as they touch their mouse / trackpad / edge. */
type DebugState = {
  connected_peer: string | null;
  has_net_tx: boolean;
  can_edge_cross: boolean;
  is_relaying: boolean;
  is_controlled: boolean;
  last_activity_s_ago: number;
  session_duration_s: number | null;
  peer_app_version: string | null;
  peer_cooldowns: Array<{ peer_id: string; remaining_s: number }>;
  connection_counts: Array<{ ip: string; in_flight: number }>;
  transition_edge: string;
  edge_dwell_ms: number;
  gaming_mode: boolean;
  bytes_in: number;
  bytes_out: number;
};

const DeveloperSection = () => {
  const { status: appStatus } = useStore();
  const connectedPeer = appStatus?.connected_peer ?? null;
  const [dbg, setDbg] = useState<DebugState | null>(null);
  const [actionMsg, setActionMsg] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null);

  // Poll the debug snapshot while connected. 1 Hz is cheap (a few lock
  // reads) and makes the flags respond visibly when the user touches
  // their mouse — handy for learning "oh, my keypress just flipped
  // is_controlled off via the kick path".
  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const v = await invoke<DebugState>('get_debug_state');
        if (!cancelled) setDbg(v);
      } catch { /* backend not ready; silent */ }
    };
    poll();
    const id = window.setInterval(poll, 1000);
    return () => { cancelled = true; window.clearInterval(id); };
  }, []);

  const flashAction = (kind: 'ok' | 'err', text: string) => {
    setActionMsg({ kind, text });
    window.setTimeout(() => setActionMsg(null), 4000);
  };

  const Dot = ({ ok }: { ok: boolean }) => (
    <span
      className="inline-block w-2 h-2 rounded-full mr-1.5 align-middle flex-shrink-0"
      style={{ background: ok ? 'var(--success)' : 'var(--danger)' }}
    />
  );

  const btnStyle = {
    background: 'var(--accent-soft-bg)',
    border: '1px solid var(--accent-soft-br)',
    color: 'var(--accent-primary)',
  };

  return (
    <Section
      title="Developer"
      className="lg:col-span-2"
      icon={
        <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round"
            d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
        </svg>
      }
    >
      <Row>
        <p className="text-[11px] mb-3 leading-relaxed" style={{ color: 'var(--text-muted)' }}>
          Live state every 1s. Use this to see WHY edge-cross isn't firing —
          both <span style={{ color: 'var(--text-secondary)' }}>Connected peer</span> and{' '}
          <span style={{ color: 'var(--text-secondary)' }}>Can edge-cross</span> must be green.
        </p>

        {!dbg ? (
          <p className="text-xs" style={{ color: 'var(--text-faint)' }}>Loading…</p>
        ) : (
          <div
            className="rounded-xl overflow-hidden grid grid-cols-2 gap-x-4 gap-y-2 text-[11px] p-3"
            style={{ background: 'var(--bg-subtle)', border: '1px solid var(--border-subtle)' }}
          >
            <div className="flex items-center">
              <Dot ok={!!dbg.connected_peer} />
              <span style={{ color: 'var(--text-muted)' }}>Connected peer</span>
              <span className="ml-auto font-mono" style={{ color: 'var(--text-secondary)' }}>
                {dbg.connected_peer ? dbg.connected_peer.slice(0, 8) : 'none'}
              </span>
            </div>
            <div className="flex items-center">
              <Dot ok={dbg.can_edge_cross} />
              <span style={{ color: 'var(--text-muted)' }}>Can edge-cross</span>
              <span className="ml-auto font-mono" style={{ color: 'var(--text-secondary)' }}>
                {dbg.can_edge_cross ? 'yes' : 'no'}
              </span>
            </div>
            <div className="flex items-center">
              <Dot ok={dbg.has_net_tx} />
              <span style={{ color: 'var(--text-muted)' }}>Outbound session (net_tx)</span>
              <span className="ml-auto font-mono" style={{ color: 'var(--text-secondary)' }}>
                {dbg.has_net_tx ? 'open' : 'none'}
              </span>
            </div>
            <div className="flex items-center">
              <Dot ok={dbg.is_relaying} />
              <span style={{ color: 'var(--text-muted)' }}>Relaying out</span>
              <span className="ml-auto font-mono" style={{ color: 'var(--text-secondary)' }}>
                {dbg.is_relaying ? 'on' : 'off'}
              </span>
            </div>
            <div className="flex items-center">
              <Dot ok={!dbg.is_controlled} />
              <span style={{ color: 'var(--text-muted)' }}>Being controlled</span>
              <span className="ml-auto font-mono" style={{ color: 'var(--text-secondary)' }}>
                {dbg.is_controlled ? 'yes' : 'no'}
              </span>
            </div>
            <div className="flex items-center">
              <span style={{ color: 'var(--text-muted)' }}>Last activity</span>
              <span className="ml-auto font-mono" style={{ color: 'var(--text-secondary)' }}>
                {dbg.last_activity_s_ago}s ago
              </span>
            </div>
            <div className="flex items-center">
              <span style={{ color: 'var(--text-muted)' }}>Session uptime</span>
              <span className="ml-auto font-mono" style={{ color: 'var(--text-secondary)' }}>
                {dbg.session_duration_s !== null ? `${dbg.session_duration_s}s` : '—'}
              </span>
            </div>
            <div className="flex items-center">
              <span style={{ color: 'var(--text-muted)' }}>Peer version</span>
              <span className="ml-auto font-mono" style={{ color: 'var(--text-secondary)' }}>
                {dbg.peer_app_version ?? 'unknown'}
              </span>
            </div>
            <div className="flex items-center">
              <span style={{ color: 'var(--text-muted)' }}>Edge / dwell</span>
              <span className="ml-auto font-mono" style={{ color: 'var(--text-secondary)' }}>
                {dbg.transition_edge} · {dbg.edge_dwell_ms}ms
              </span>
            </div>
            <div className="flex items-center">
              <span style={{ color: 'var(--text-muted)' }}>Bytes in / out</span>
              <span className="ml-auto font-mono" style={{ color: 'var(--text-secondary)' }}>
                {dbg.bytes_in.toLocaleString()} / {dbg.bytes_out.toLocaleString()}
              </span>
            </div>
            <div className="flex items-center">
              <Dot ok={!dbg.gaming_mode} />
              <span style={{ color: 'var(--text-muted)' }}>Gaming mode</span>
              <span className="ml-auto font-mono" style={{ color: 'var(--text-secondary)' }}>
                {dbg.gaming_mode ? 'on (edge-cross disabled)' : 'off'}
              </span>
            </div>
            {dbg.peer_cooldowns.length > 0 && (
              <div className="col-span-2 mt-1">
                <span style={{ color: 'var(--danger)' }}>Cooldowns active:</span>
                <span className="ml-1.5 font-mono" style={{ color: 'var(--text-secondary)' }}>
                  {dbg.peer_cooldowns.map((c) => `${c.peer_id.slice(0, 8)} (${c.remaining_s}s)`).join(', ')}
                </span>
              </div>
            )}
          </div>
        )}

        <div className="flex flex-wrap gap-2 mt-3">
          <button
            onClick={async () => {
              try {
                const n = await invoke<number>('clear_all_cooldowns');
                flashAction('ok', n > 0 ? `Cleared ${n} cooldown${n === 1 ? '' : 's'}` : 'No cooldowns to clear');
              } catch (e) { flashAction('err', String(e)); }
            }}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold transition-all active:scale-95"
            style={btnStyle}
          >
            Clear cooldowns
          </button>
          <button
            onClick={async () => {
              if (!connectedPeer) {
                flashAction('err', 'Not connected to a peer');
                return;
              }
              try {
                await invoke('force_dial_peer', { peerId: connectedPeer });
                flashAction('ok', 'Dialing peer as outbound client…');
              } catch (e) { flashAction('err', String(e)); }
            }}
            disabled={!connectedPeer || (dbg?.has_net_tx ?? false)}
            title={
              !connectedPeer ? 'Connect to a peer first' :
              dbg?.has_net_tx ? 'Already have an outbound session' :
              'Force Mac to also dial the peer so edge-cross works both ways'
            }
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold transition-all active:scale-95 disabled:opacity-40 disabled:active:scale-100"
            style={btnStyle}
          >
            Open outbound session
          </button>
          <button
            onClick={async () => {
              try {
                await invoke('disconnect');
                flashAction('ok', 'Disconnect signaled');
              } catch (e) { flashAction('err', String(e)); }
            }}
            disabled={!connectedPeer}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold transition-all active:scale-95 disabled:opacity-40"
            style={{
              background: 'var(--bg-subtle)',
              border: '1px solid var(--border-subtle)',
              color: 'var(--danger)',
            }}
          >
            Force disconnect
          </button>
        </div>

        <AnimatePresence>
          {actionMsg && (
            <motion.p
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: 'auto' }}
              exit={{ opacity: 0, height: 0 }}
              className="text-[11px] mt-2.5 leading-relaxed"
              style={{ color: actionMsg.kind === 'ok' ? 'var(--success)' : 'var(--danger)' }}
            >
              {actionMsg.text}
            </motion.p>
          )}
        </AnimatePresence>
      </Row>
    </Section>
  );
};

/* ── Diagnostics section ──
   Three buttons: reveal the log in Finder/Explorer, copy last 500 lines to
   clipboard for pasting into a support thread, and export a structured JSON
   bundle (OS + settings + log tails) to the Desktop. All three invoke
   Rust-side commands that redact secrets. Uses a local transient status
   line instead of a toast system — the project doesn't have one and
   inventing one just for this section is out of scope. */
const DiagnosticsSection = () => {
  const { status: appStatus } = useStore();
  const [status, setStatus] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null);
  const [pullBusy, setPullBusy] = useState(false);
  const [peerAppVersion, setPeerAppVersion] = useState<string | null>(null);
  const flash = (kind: 'ok' | 'err', text: string) => {
    setStatus({ kind, text });
    window.setTimeout(() => setStatus(null), 4000);
  };

  const connectedPeer = appStatus?.connected_peer ?? null;

  // Refresh peer app version whenever a session begins / ends — the command
  // is cheap (a single lock read) so we can just poll while connected.
  useEffect(() => {
    if (!connectedPeer) {
      setPeerAppVersion(null);
      return;
    }
    let cancelled = false;
    const poll = async () => {
      try {
        const v = await invoke<string | null>('get_peer_app_version');
        if (!cancelled) setPeerAppVersion(v ?? null);
      } catch { /* not connected yet; try again on next tick */ }
    };
    poll();
    const id = window.setInterval(poll, 2000);
    return () => { cancelled = true; window.clearInterval(id); };
  }, [connectedPeer]);

  // Peer must be on 0.3.8+ for LogRequest/LogShare variants to deserialize.
  // Earlier peers drop the session on unknown variant, so the button stays
  // disabled until we've confirmed the peer is new enough.
  const peerSupportsLogPull = (() => {
    if (!peerAppVersion) return false;
    const parts = peerAppVersion.split('.').map((p) => parseInt(p, 10));
    if (parts.length < 3 || parts.some(isNaN)) return false;
    const [maj, min, pat] = parts;
    return maj > 0 || (maj === 0 && (min > 3 || (min === 3 && pat >= 8)));
  })();

  const btnStyle = {
    background: 'var(--accent-soft-bg)',
    border: '1px solid var(--accent-soft-br)',
    color: 'var(--accent-primary)',
  };

  const disabledStyle = {
    background: 'var(--bg-subtle)',
    border: '1px solid var(--border-subtle)',
    color: 'var(--text-ghost)',
    cursor: 'not-allowed',
  };

  return (
    <Section
      title="Diagnostics"
      className="lg:col-span-2"
      icon={
        <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round"
            d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
        </svg>
      }
    >
      <Row noDivider>
        <p className="text-[11px] mb-3 leading-relaxed" style={{ color: 'var(--text-muted)' }}>
          When something goes wrong, these make it easy to share what happened without the terminal.
        </p>
        <div className="flex flex-wrap gap-2">
          <button
            onClick={async () => {
              try { await invoke('open_log_file'); flash('ok', 'Revealed in Finder'); }
              catch (e) { flash('err', String(e)); }
            }}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold transition-all active:scale-95"
            style={btnStyle}
          >
            View log file
          </button>
          <button
            onClick={async () => {
              try {
                const n = await invoke<number>('copy_log_to_clipboard');
                flash('ok', `Copied ${n.toLocaleString()} bytes to clipboard`);
              } catch (e) { flash('err', String(e)); }
            }}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold transition-all active:scale-95"
            style={btnStyle}
          >
            Copy log to clipboard
          </button>
          <button
            onClick={async () => {
              try {
                const p = await invoke<string>('export_diagnostics_bundle', { peerLog: null });
                flash('ok', `Saved ${p.split('/').pop() ?? p}`);
              } catch (e) { flash('err', String(e)); }
            }}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold transition-all active:scale-95"
            style={btnStyle}
          >
            Export diagnostics bundle
          </button>
          <button
            onClick={async () => {
              if (!connectedPeer || !peerSupportsLogPull || pullBusy) return;
              setPullBusy(true);
              flash('ok', 'Asking peer… accept the modal on their machine');
              try {
                const localName = appStatus?.device_name ?? 'this device';
                const peerLog = await invoke<string>('request_peer_logs', {
                  localDeviceName: localName,
                });
                if (!peerLog) {
                  flash('err', 'Peer declined or did not respond');
                } else {
                  const p = await invoke<string>('export_diagnostics_bundle', { peerLog });
                  flash('ok', `Saved bundle with peer log · ${p.split('/').pop() ?? p}`);
                }
              } catch (e) { flash('err', String(e)); }
              finally { setPullBusy(false); }
            }}
            disabled={!connectedPeer || !peerSupportsLogPull || pullBusy}
            title={
              !connectedPeer ? 'Connect to a peer first' :
              !peerSupportsLogPull ? (peerAppVersion ? `Peer is on v${peerAppVersion}; requires v0.3.8+` : 'Peer version unknown yet') :
              'Request the peer\u2019s log tail and save a combined bundle'
            }
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold transition-all active:scale-95 disabled:active:scale-100"
            style={(!connectedPeer || !peerSupportsLogPull || pullBusy) ? disabledStyle : btnStyle}
          >
            {pullBusy ? 'Waiting on peer\u2026' : 'Pull peer logs + export'}
          </button>
        </div>
        <AnimatePresence>
          {status && (
            <motion.p
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: 'auto' }}
              exit={{ opacity: 0, height: 0 }}
              className="text-[11px] mt-2.5 leading-relaxed"
              style={{ color: status.kind === 'ok' ? 'var(--success)' : 'var(--danger)' }}
            >
              {status.text}
            </motion.p>
          )}
        </AnimatePresence>
      </Row>
    </Section>
  );
};

/* ── Hotkeys & Input section ──
   Groups: mouse sensitivity, transition edge, release hotkey, two
   input-behaviour toggles, and the gaming-mode switch-hotkey editor. All
   controls write through the parent's `update()` helper so there's a single
   settings-persistence path. */
const MAX_SWITCH_HOTKEYS = 9;
const SWITCH_HOTKEY_VALUES = [
  'F1','F2','F3','F4','F5','F6','F7','F8','F9','F10','F11','F12',
];
const EDGE_VALUES: Array<Settings['transition_edge']> = ['left', 'right', 'top', 'bottom'];
const RELEASE_HOTKEY_OPTIONS: Array<{ value: string; label: string }> = [
  { value: 'ctrl+ctrl',   label: 'Ctrl × 2' },
  { value: 'shift+shift', label: 'Shift × 2' },
  { value: 'alt+alt',     label: 'Alt × 2' },
  { value: 'caps_lock',   label: 'Caps Lock' },
];

const HotkeysAndInputSection = ({
  settings,
  update,
}: {
  settings: Settings;
  update: (patch: Partial<Settings>) => Promise<void>;
}) => {
  // F-key capture: clicking "Record" focuses a hidden input that swallows the
  // next F1–F12 keydown. macOS users may have F-keys remapped to media
  // controls by the OS (Fn-key row), so we also show a manual text input
  // as a reliable fallback.
  const [recording, setRecording] = useState(false);
  const [manualInput, setManualInput] = useState('');
  const recordRef = useRef<HTMLInputElement | null>(null);

  const current = settings.switch_hotkeys ?? [];
  const canAdd = current.length < MAX_SWITCH_HOTKEYS;

  const addHotkey = (name: string) => {
    const v = name.trim().toUpperCase();
    if (!SWITCH_HOTKEY_VALUES.includes(v)) return;
    if (current.includes(v)) return;
    if (!canAdd) return;
    update({ switch_hotkeys: [...current, v] });
  };

  const removeHotkey = (name: string) => {
    update({ switch_hotkeys: current.filter((h) => h !== name) });
  };

  const onRecordKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    // Only F1–F12 are valid swap hotkeys server-side. Anything else is a
    // no-op — user sees the button snap back to "Record" and can retry.
    e.preventDefault();
    const k = e.key;
    if (/^F\d{1,2}$/.test(k) && SWITCH_HOTKEY_VALUES.includes(k)) {
      addHotkey(k);
    }
    setRecording(false);
  };

  useEffect(() => {
    if (recording) recordRef.current?.focus();
  }, [recording]);

  return (
    <Section
      title="Hotkeys & Input"
      className="lg:col-span-2"
      icon={
        <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M12 18h.01M8 21h8a2 2 0 002-2V5a2 2 0 00-2-2H8a2 2 0 00-2 2v14a2 2 0 002 2z" />
        </svg>
      }
    >
      {/* Mouse sensitivity */}
      <Row>
        <div className="flex items-start justify-between gap-4 mb-3">
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium" style={{ color: 'var(--text-secondary)' }}>
              Mouse sensitivity
            </p>
            <p className="text-[11px] mt-0.5 leading-relaxed" style={{ color: 'var(--text-faint)' }}>
              Multiplies movement before it's sent to the other machine. Raise it if your mouse feels slow on the remote side due to a DPI mismatch.
            </p>
          </div>
          <span
            className="text-xs font-mono font-bold px-2.5 py-1 rounded-lg flex-shrink-0"
            style={{
              background: 'var(--accent-soft-bg)',
              border: '1px solid var(--accent-soft-br)',
              color: 'var(--accent-primary)',
            }}
          >
            {(settings.mouse_sensitivity ?? 1.0).toFixed(1)}×
          </span>
        </div>
        <input
          type="range"
          min={0.1}
          max={5.0}
          step={0.1}
          value={settings.mouse_sensitivity ?? 1.0}
          onChange={(e) => update({ mouse_sensitivity: Number(e.target.value) })}
          className="w-full"
          style={{ accentColor: 'var(--accent-primary)' }}
        />
        <div className="flex justify-between mt-1 text-[10px]" style={{ color: 'var(--text-faint)' }}>
          <span>0.1×</span>
          <span>1.0×</span>
          <span>5.0×</span>
        </div>
      </Row>

      {/* Transition edge */}
      <Row>
        <div className="flex items-start justify-between gap-4 mb-2">
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium" style={{ color: 'var(--text-secondary)' }}>
              Transition edge
            </p>
            <p className="text-[11px] mt-0.5 leading-relaxed" style={{ color: 'var(--text-faint)' }}>
              Which screen edge sends the cursor to the other machine.
            </p>
          </div>
        </div>
        <div
          className="flex items-center rounded-xl p-0.5 flex-wrap gap-0.5"
          style={{
            background: 'var(--bg-subtle)',
            border: '1px solid var(--border-subtle)',
          }}
        >
          {EDGE_VALUES.map((edge) => {
            const active = settings.transition_edge === edge;
            return (
              <button
                key={edge}
                onClick={() => update({ transition_edge: edge })}
                className="relative flex-1 px-2 py-1 text-[11px] font-semibold transition-colors capitalize"
                style={{ color: active ? 'white' : 'var(--text-muted)', minWidth: 52 }}
              >
                {active && (
                  <motion.div
                    layoutId="edge-pill"
                    className="absolute inset-0 rounded-lg"
                    style={{
                      background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                      boxShadow: '0 2px 8px rgba(99,102,241,0.35)',
                    }}
                    transition={{ type: 'spring', stiffness: 500, damping: 34 }}
                  />
                )}
                <span className="relative z-10">{edge}</span>
              </button>
            );
          })}
        </div>
      </Row>

      {/* Release hotkey */}
      <Row>
        <div className="flex items-start justify-between gap-4 mb-2">
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium" style={{ color: 'var(--text-secondary)' }}>
              Release hotkey
            </p>
            <p className="text-[11px] mt-0.5 leading-relaxed" style={{ color: 'var(--text-faint)' }}>
              Emergency way to yank control back to this machine. Esc also always works.
            </p>
          </div>
        </div>
        <div
          className="flex items-center rounded-xl p-0.5 flex-wrap gap-0.5"
          style={{
            background: 'var(--bg-subtle)',
            border: '1px solid var(--border-subtle)',
          }}
        >
          {RELEASE_HOTKEY_OPTIONS.map((opt) => {
            const active = settings.hotkey_release === opt.value;
            return (
              <button
                key={opt.value}
                onClick={() => update({ hotkey_release: opt.value })}
                className="relative flex-1 px-2 py-1 text-[11px] font-semibold transition-colors"
                style={{ color: active ? 'white' : 'var(--text-muted)', minWidth: 80 }}
              >
                {active && (
                  <motion.div
                    layoutId="release-pill"
                    className="absolute inset-0 rounded-lg"
                    style={{
                      background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                      boxShadow: '0 2px 8px rgba(99,102,241,0.35)',
                    }}
                    transition={{ type: 'spring', stiffness: 500, damping: 34 }}
                  />
                )}
                <span className="relative z-10">{opt.label}</span>
              </button>
            );
          })}
        </div>
      </Row>

      {/* Behaviour toggles */}
      <Row>
        <Toggle
          label="Hide cursor while controlling"
          description="Make this machine's cursor invisible while you're driving the remote, so two cursors don't compete visually."
          checked={settings.hide_cursor_during_relay ?? true}
          onChange={(v) => update({ hide_cursor_during_relay: v })}
        />
      </Row>
      <Row>
        <Toggle
          label="Auto gaming mode"
          description="Turn gaming mode on automatically when a fullscreen app is in the foreground."
          checked={settings.auto_gaming_mode ?? true}
          onChange={(v) => update({ auto_gaming_mode: v })}
        />
      </Row>

      {/* Switch hotkeys (gaming-mode only) */}
      <Row noDivider>
        <div className="flex items-start justify-between gap-4 mb-2">
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium" style={{ color: 'var(--text-secondary)' }}>
              Swap-control hotkeys (gaming mode)
            </p>
            <p className="text-[11px] mt-0.5 leading-relaxed" style={{ color: 'var(--text-faint)' }}>
              While gaming mode is on, pressing any of these toggles which machine the mouse drives. F1–F12 only; up to {MAX_SWITCH_HOTKEYS}.
            </p>
          </div>
          <span
            className="text-xs font-mono font-bold px-2.5 py-1 rounded-lg flex-shrink-0"
            style={{
              background: 'var(--accent-soft-bg)',
              border: '1px solid var(--accent-soft-br)',
              color: 'var(--accent-primary)',
            }}
          >
            {current.length}/{MAX_SWITCH_HOTKEYS}
          </span>
        </div>

        {current.length > 0 && (
          <div className="flex flex-wrap gap-1.5 mb-2">
            {current.map((h) => (
              <span
                key={h}
                className="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-mono font-bold"
                style={{
                  background: 'var(--bg-subtle)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-secondary)',
                }}
              >
                {h}
                <button
                  onClick={() => removeHotkey(h)}
                  className="ml-0.5 w-4 h-4 rounded-full flex items-center justify-center transition-colors"
                  style={{ color: 'var(--text-ghost)' }}
                  title={`Remove ${h}`}
                  aria-label={`Remove ${h}`}
                >
                  <svg className="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </button>
              </span>
            ))}
          </div>
        )}

        <div className="flex items-center gap-2 flex-wrap">
          <button
            onClick={() => setRecording(true)}
            disabled={!canAdd}
            className="text-xs px-2.5 py-1.5 rounded-lg font-semibold transition-all disabled:opacity-40"
            style={{
              background: recording ? 'var(--accent-soft-bg)' : 'var(--bg-subtle)',
              border: `1px solid ${recording ? 'var(--accent-soft-br)' : 'var(--border-subtle)'}`,
              color: recording ? 'var(--accent-primary)' : 'var(--text-muted)',
            }}
          >
            {recording ? 'Press F1–F12…' : 'Record'}
          </button>
          {/* Hidden input that captures the next keydown while recording. */}
          <input
            ref={recordRef}
            type="text"
            value=""
            readOnly
            onKeyDown={onRecordKeyDown}
            onBlur={() => setRecording(false)}
            tabIndex={-1}
            aria-hidden={!recording}
            className="sr-only"
          />
          <span className="text-[10px]" style={{ color: 'var(--text-faint)' }}>or</span>
          <input
            type="text"
            value={manualInput}
            onChange={(e) => setManualInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                addHotkey(manualInput);
                setManualInput('');
              }
            }}
            placeholder="type F9"
            disabled={!canAdd}
            className="rounded-lg px-2 py-1 text-xs font-mono focus:outline-none disabled:opacity-40"
            style={{
              background: 'var(--bg-input)',
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-secondary)',
              width: 88,
            }}
          />
          <button
            onClick={() => {
              addHotkey(manualInput);
              setManualInput('');
            }}
            disabled={!canAdd || !manualInput.trim()}
            className="text-xs px-2.5 py-1.5 rounded-lg font-semibold transition-all disabled:opacity-40"
            style={{
              background: 'var(--bg-subtle)',
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-muted)',
            }}
          >
            Add
          </button>
        </div>
      </Row>
    </Section>
  );
};

/* ── Known device row ── */
const KnownDeviceRow = ({
  device,
  onForget,
}: {
  device: KnownDevice;
  onForget: (id: string) => void;
}) => {
  const [confirming, setConfirming] = useState(false);

  return (
    <div className="px-4 py-3 flex items-center gap-3" style={{ borderBottom: '1px solid var(--divider)' }}>
      {/* Avatar */}
      <div
        className="w-8 h-8 rounded-xl flex items-center justify-center flex-shrink-0 text-sm font-bold"
        style={{
          background: 'var(--accent-soft-bg)',
          border: '1px solid var(--accent-soft-br)',
          color: 'var(--accent-primary)',
        }}
      >
        {device.name.charAt(0).toUpperCase()}
      </div>

      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium truncate" style={{ color: 'var(--text-secondary)' }}>
          {device.name}
        </p>
        <p className="text-[10px] font-mono truncate" style={{ color: 'var(--text-faint)' }}>
          {device.addr}
        </p>
      </div>

      <div className="flex-shrink-0">
        <AnimatePresence mode="wait">
          {confirming ? (
            <motion.div
              key="confirm"
              initial={{ opacity: 0, scale: 0.9 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.9 }}
              className="flex items-center gap-1.5"
            >
              <button
                onClick={() => setConfirming(false)}
                className="text-xs px-2 py-1 rounded-lg transition-all"
                style={{ color: 'var(--text-muted)', background: 'var(--bg-subtle)' }}
              >
                Keep
              </button>
              <button
                onClick={() => onForget(device.id)}
                className="text-xs px-2 py-1 rounded-lg transition-all font-semibold"
                style={{ color: 'var(--danger)', background: 'rgba(239,68,68,0.12)', border: '1px solid rgba(239,68,68,0.28)' }}
              >
                Forget
              </button>
            </motion.div>
          ) : (
            <motion.button
              key="x"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              onClick={() => setConfirming(true)}
              className="w-7 h-7 rounded-lg flex items-center justify-center transition-all"
              style={{ color: 'var(--text-ghost)', background: 'var(--bg-subtle)' }}
              title="Forget device"
            >
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </motion.button>
          )}
        </AnimatePresence>
      </div>
    </div>
  );
};

export const SettingsPage = () => {
  const { settings, setSettings, status } = useStore();
  const [knownDevices, setKnownDevices] = useState<KnownDevice[]>([]);
  const [relayInput, setRelayInput] = useState('');
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [updateMsg, setUpdateMsg] = useState<string | null>(null);
  const [updateOk, setUpdateOk] = useState(false);

  const [bandwidth, setBandwidth] = useState<BandwidthStats | null>(null);
  const [auditLog, setAuditLog] = useState<AuditEntry[]>([]);
  const [auditLoading, setAuditLoading] = useState(false);
  const [confirmClearAudit, setConfirmClearAudit] = useState(false);
  // Pulled from Tauri at mount so the About section always reflects the
  // *actual* bundled version, not a stringly-typed constant that drifts.
  const [appVersion, setAppVersion] = useState<string>('');

  const connectedPeer = status?.connected_peer ?? null;

  useEffect(() => {
    if (settings) setRelayInput(settings.relay_url ?? '');
    invoke<KnownDevice[]>('get_known_devices').then(setKnownDevices).catch(() => {});
  }, [settings?.relay_url]);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => setAppVersion(''));
  }, []);

  // Poll bandwidth while a peer is connected.
  useEffect(() => {
    if (!connectedPeer) {
      setBandwidth(null);
      return;
    }
    let cancelled = false;
    const tick = async () => {
      try {
        const b = await invoke<BandwidthStats>('get_bandwidth');
        if (!cancelled) setBandwidth(b);
      } catch {
        // backend may not be ready yet — silently ignore
      }
    };
    tick();
    const id = window.setInterval(tick, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [connectedPeer]);

  // Load audit log on mount (and refresh occasionally on connect/disconnect).
  const loadAuditLog = async () => {
    setAuditLoading(true);
    try {
      const entries = await invoke<AuditEntry[]>('get_audit_log');
      setAuditLog(Array.isArray(entries) ? entries : []);
    } catch {
      // backend may not be ready yet — leave list empty
    } finally {
      setAuditLoading(false);
    }
  };

  useEffect(() => {
    loadAuditLog();
  }, []);

  useEffect(() => {
    loadAuditLog();
  }, [connectedPeer]);

  const sentShare = useMemo(() => {
    if (!bandwidth) return 0.5;
    const total = bandwidth.bytes_sent + bandwidth.bytes_received;
    if (total <= 0) return 0.5;
    return bandwidth.bytes_sent / total;
  }, [bandwidth]);

  const recentAudit = useMemo(() => auditLog.slice(0, 20), [auditLog]);

  const handleClearAudit = async () => {
    try {
      await invoke('clear_audit_log');
      setAuditLog([]);
    } catch (e) {
      console.error('clear_audit_log failed', e);
    } finally {
      setConfirmClearAudit(false);
    }
  };

  if (!settings) return null;

  const update = async (patch: Partial<Settings>) => {
    const next = { ...settings, ...patch };
    setSettings(next);
    await invoke('update_settings', { settings: next }).catch(console.error);
  };

  const handleForget = async (id: string) => {
    await invoke('forget_device', { deviceId: id }).catch(console.error);
    setKnownDevices((prev) => prev.filter((d) => d.id !== id));
  };

  const checkUpdate = async () => {
    setCheckingUpdate(true);
    setUpdateMsg(null);
    setUpdateOk(false);
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      const result = await check();
      if (result?.available) {
        setUpdateMsg(`v${result.version} available — downloading…`);
        setUpdateOk(false);
        await result.downloadAndInstall();
        const { relaunch } = await import('@tauri-apps/plugin-process');
        await relaunch();
      } else {
        setUpdateOk(true);
        setUpdateMsg("You're on the latest version");
        setTimeout(() => setUpdateMsg(null), 4000);
      }
    } catch {
      setUpdateOk(false);
      setUpdateMsg('Could not check for updates');
      setTimeout(() => setUpdateMsg(null), 4000);
    } finally {
      setCheckingUpdate(false);
    }
  };

  return (
    <div className="flex flex-col flex-1 overflow-y-auto px-5 py-4 gap-4 pb-3">

      {/* Two-column grid: short sections flow side-by-side at wider widths */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">

      {/* ── General ── */}
      <Section
        title="General"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4" />
          </svg>
        }
      >
        <Row>
          <Toggle
            label="Launch on startup"
            description="Start MultiMouse automatically when you log in"
            checked={settings.launch_on_startup}
            onChange={(v) => update({ launch_on_startup: v })}
          />
        </Row>
        <Row>
          <Toggle
            label="Auto-reconnect"
            description="Reconnect automatically after sleep or network change"
            checked={settings.auto_reconnect ?? true}
            onChange={(v) => update({ auto_reconnect: v })}
          />
        </Row>
        <Row>
          <Toggle
            label="Privacy blur while controlling"
            description="Blur this window when your cursor is driving a remote device so onlookers can't see the other screen's reflection"
            checked={settings.privacy_blur_on_relay ?? true}
            onChange={(v) => update({ privacy_blur_on_relay: v })}
          />
        </Row>
        <Row noDivider>
          <div className="flex items-center justify-between gap-4">
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium" style={{ color: 'var(--text-secondary)' }}>Theme</p>
              <p className="text-[11px] mt-0.5 leading-relaxed" style={{ color: 'var(--text-faint)' }}>
                Switch between the default dark chrome and a bright VSCode-style light palette
              </p>
            </div>
            <div
              className="flex items-center rounded-xl p-0.5 flex-shrink-0"
              style={{
                background: 'var(--bg-subtle)',
                border: '1px solid var(--border-subtle)',
              }}
            >
              {(['dark', 'light'] as const).map((t) => {
                const active = settings.theme === t;
                return (
                  <button
                    key={t}
                    onClick={() => update({ theme: t })}
                    className="relative px-3 py-1 text-xs font-semibold transition-colors"
                    style={{
                      color: active ? 'white' : 'var(--text-muted)',
                    }}
                  >
                    {active && (
                      <motion.div
                        layoutId="theme-pill"
                        className="absolute inset-0 rounded-lg"
                        style={{
                          background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                          boxShadow: '0 2px 8px rgba(99,102,241,0.35)',
                        }}
                        transition={{ type: 'spring', stiffness: 500, damping: 34 }}
                      />
                    )}
                    <span className="relative z-10 capitalize">{t}</span>
                  </button>
                );
              })}
            </div>
          </div>
        </Row>
      </Section>

      {/* ── Security ── */}
      <Section
        title="Security"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round"
              d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
          </svg>
        }
      >
        <Row noDivider>
          <div className="flex items-start justify-between gap-4 mb-2">
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium" style={{ color: 'var(--text-secondary)' }}>
                Idle auto-lock
              </p>
              <p className="text-[11px] mt-0.5 leading-relaxed" style={{ color: 'var(--text-faint)' }}>
                Drop remote control after a period of inactivity
              </p>
            </div>
            <span
              className="text-xs font-mono font-bold px-2.5 py-1 rounded-lg flex-shrink-0"
              style={{
                background: 'var(--accent-soft-bg)',
                border: '1px solid var(--accent-soft-br)',
                color: 'var(--accent-primary)',
              }}
            >
              {(settings.idle_lock_minutes ?? 0) === 0
                ? 'Off'
                : `${settings.idle_lock_minutes} min`}
            </span>
          </div>
          <div
            className="flex items-center rounded-xl p-0.5 flex-wrap gap-0.5"
            style={{
              background: 'var(--bg-subtle)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            {[0, 5, 10, 15, 30, 60].map((mins) => {
              const active = (settings.idle_lock_minutes ?? 0) === mins;
              return (
                <button
                  key={mins}
                  onClick={() => update({ idle_lock_minutes: mins })}
                  className="relative flex-1 px-2 py-1 text-[11px] font-semibold transition-colors"
                  style={{ color: active ? 'white' : 'var(--text-muted)', minWidth: 38 }}
                >
                  {active && (
                    <motion.div
                      layoutId="idlelock-pill"
                      className="absolute inset-0 rounded-lg"
                      style={{
                        background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                        boxShadow: '0 2px 8px rgba(99,102,241,0.35)',
                      }}
                      transition={{ type: 'spring', stiffness: 500, damping: 34 }}
                    />
                  )}
                  <span className="relative z-10">{mins === 0 ? 'Off' : `${mins}m`}</span>
                </button>
              );
            })}
          </div>
        </Row>
      </Section>

      {/* ── Edge switching ── */}
      <Section
        title="Edge Switching"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M13 5l7 7-7 7M5 5l7 7-7 7" />
          </svg>
        }
      >
        <Row>
          <Toggle
            label="Gaming mode"
            description="Disable edge-cross so a hard flick to the edge mid-match won't yank the cursor to the other computer. Hotkey release still works. Toggle with Pause/Break."
            checked={settings.gaming_mode ?? false}
            onChange={(v) => update({ gaming_mode: v })}
          />
        </Row>
        <Row noDivider>
          <div className="flex items-start justify-between gap-4 mb-3">
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium" style={{ color: 'var(--text-secondary)' }}>
                Edge dwell time
              </p>
              <p className="text-[11px] mt-0.5 leading-relaxed" style={{ color: 'var(--text-faint)' }}>
                Time to hold cursor at edge before switching
              </p>
            </div>
            <span
              className="text-xs font-mono font-bold px-2.5 py-1 rounded-lg flex-shrink-0"
              style={{
                background: 'var(--accent-soft-bg)',
                border: '1px solid var(--accent-soft-br)',
                color: 'var(--accent-primary)',
              }}
            >
              {settings.edge_dwell_ms ?? 150} ms
            </span>
          </div>
          <input
            type="range"
            min={50}
            max={500}
            step={25}
            value={settings.edge_dwell_ms ?? 150}
            onChange={(e) => update({ edge_dwell_ms: Number(e.target.value) })}
            className="w-full accent-indigo-400"
            style={{ accentColor: 'var(--accent-primary)' }}
          />
          <div className="flex justify-between mt-1 text-[10px]" style={{ color: 'var(--text-faint)' }}>
            <span>50 ms</span>
            <span>500 ms</span>
          </div>
        </Row>
      </Section>

      {/* ── Hotkeys & Input ── */}
      <HotkeysAndInputSection settings={settings} update={update} />

      {/* ── Internet relay ── */}
      <Section
        title="Internet Relay"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round"
              d="M3.055 11H5a2 2 0 012 2v1a2 2 0 002 2 2 2 0 012 2v2.945M8 3.935V5.5A2.5 2.5 0 0010.5 8h.5a2 2 0 012 2 2 2 0 104 0 2 2 0 012-2h1.064M15 20.488V18a2 2 0 012-2h3.064" />
          </svg>
        }
      >
        <Row noDivider>
          <p className="text-[11px] mb-2.5 leading-relaxed" style={{ color: 'var(--text-muted)' }}>
            Self-host the relay binary and paste its address here to connect over the internet without being on the same Wi-Fi.
          </p>
          <input
            type="text"
            value={relayInput}
            onChange={(e) => setRelayInput(e.target.value)}
            onBlur={() => update({ relay_url: relayInput.trim() })}
            onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
            placeholder="relay.yourserver.com:57173"
            className="w-full rounded-xl px-3 py-2.5 text-xs font-mono focus:outline-none transition-all"
            style={{
              background: 'var(--bg-input)',
              border: `1.5px solid ${relayInput ? 'var(--border-strong)' : 'var(--border-subtle)'}`,
              color: 'var(--text-secondary)',
            }}
          />
        </Row>
      </Section>

      {/* ── Paired Devices ── */}
      <Section
        title={`Paired Devices${knownDevices.length > 0 ? ` · ${knownDevices.length}` : ''}`}
        className="lg:col-span-2"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round"
              d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
          </svg>
        }
      >
        {knownDevices.length === 0 ? (
          <div className="px-4 py-5 flex flex-col items-center gap-1.5">
            <svg className="w-7 h-7 mb-1" fill="none" viewBox="0 0 24 24" stroke="var(--text-ghost)" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round"
                d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
            </svg>
            <p className="text-xs" style={{ color: 'var(--text-faint)' }}>No paired devices yet</p>
            <p className="text-[10px] text-center" style={{ color: 'var(--text-ghost)' }}>
              Accept a connection request to pair a device — it'll auto-connect without a PIN next time.
            </p>
          </div>
        ) : (
          <>
            <AnimatePresence>
              {knownDevices.map((d) => (
                <motion.div
                  key={d.id}
                  layout
                  exit={{ opacity: 0, height: 0, overflow: 'hidden' }}
                  transition={{ duration: 0.18 }}
                >
                  <KnownDeviceRow device={d} onForget={handleForget} />
                </motion.div>
              ))}
            </AnimatePresence>
            <div className="px-4 py-2.5">
              <p className="text-[10px]" style={{ color: 'var(--text-ghost)' }}>
                Paired devices reconnect automatically without a PIN.
              </p>
            </div>
          </>
        )}
      </Section>

      {/* ── Session Stats (only while connected) ── */}
      {connectedPeer && (
        <Section
          title="Session Stats"
          className="lg:col-span-2"
          icon={
            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round"
                d="M13 7h8m0 0v8m0-8l-8 8-4-4-6 6" />
            </svg>
          }
        >
          <Row>
            <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
              {[
                { label: 'Sent', value: formatBytes(bandwidth?.bytes_sent ?? 0), color: 'var(--accent-primary)' },
                { label: 'Received', value: formatBytes(bandwidth?.bytes_received ?? 0), color: 'var(--success)' },
                {
                  label: 'Total',
                  value: formatBytes(
                    (bandwidth?.bytes_sent ?? 0) + (bandwidth?.bytes_received ?? 0),
                  ),
                  color: '#fbbf24',
                },
                { label: 'Uptime', value: formatUptime(bandwidth?.uptime_secs ?? 0), color: '#60a5fa' },
              ].map((tile) => (
                <div
                  key={tile.label}
                  className="rounded-xl px-4 py-3"
                  style={{
                    background: 'var(--bg-subtle)',
                    border: '1px solid var(--border-subtle)',
                  }}
                >
                  <p
                    className="text-[10px] font-bold uppercase tracking-widest mb-1.5"
                    style={{ color: 'var(--text-muted)' }}
                  >
                    {tile.label}
                  </p>
                  <p
                    className="text-sm font-mono font-bold"
                    style={{ color: tile.color }}
                  >
                    {tile.value}
                  </p>
                </div>
              ))}
            </div>
          </Row>
          <Row noDivider>
            <p
              className="text-[10px] font-bold uppercase tracking-widest mb-1.5"
              style={{ color: 'var(--text-faint)' }}
            >
              Ratio
            </p>
            <div
              className="w-full h-2 rounded-full overflow-hidden flex"
              style={{ background: 'var(--bg-subtle)' }}
            >
              <div
                className="h-full transition-all"
                style={{
                  width: `${Math.round(sentShare * 100)}%`,
                  background: 'linear-gradient(90deg, #6366f1, #a855f7)',
                }}
              />
              <div
                className="h-full transition-all"
                style={{
                  width: `${Math.round((1 - sentShare) * 100)}%`,
                  background: 'linear-gradient(90deg, #10b981, #34d399)',
                }}
              />
            </div>
            <div className="flex justify-between mt-1 text-[10px]" style={{ color: 'var(--text-faint)' }}>
              <span>Sent</span>
              <span>Received</span>
            </div>
          </Row>
        </Section>
      )}

      {/* ── Activity Log ── */}
      <Section
        title="Activity Log"
        className="lg:col-span-2"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round"
              d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
        }
      >
        {recentAudit.length === 0 ? (
          <div className="px-4 py-5 flex flex-col items-center gap-1.5">
            <svg className="w-7 h-7 mb-1" fill="none" viewBox="0 0 24 24" stroke="var(--text-ghost)" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round"
                d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            <p className="text-xs" style={{ color: 'var(--text-faint)' }}>
              {auditLoading ? 'Loading…' : 'No activity yet'}
            </p>
          </div>
        ) : (
          <>
            <div className="max-h-56 overflow-y-auto">
              {recentAudit.map((entry, idx) => (
                <div
                  key={`${entry.timestamp}-${idx}`}
                  className="px-4 py-2.5 flex items-center gap-3"
                  style={{
                    borderBottom:
                      idx === recentAudit.length - 1
                        ? 'none'
                        : '1px solid var(--divider)',
                  }}
                >
                  <span
                    className="w-2 h-2 rounded-full flex-shrink-0"
                    style={{ background: auditDotColor(entry.action) }}
                    aria-label={auditDotEmoji(entry.action)}
                  />
                  <div className="flex-1 min-w-0">
                    <p className="text-xs font-medium truncate" style={{ color: 'var(--text-secondary)' }}>
                      {entry.peer_name || 'Unknown device'}
                      <span className="mx-1.5" style={{ color: 'var(--text-faint)' }}>·</span>
                      <span style={{ color: 'var(--text-muted)' }}>{auditLabel(entry.action)}</span>
                    </p>
                    {entry.details && (
                      <p className="text-[10px] mt-0.5 truncate" style={{ color: 'var(--text-faint)' }}>
                        {entry.details}
                      </p>
                    )}
                  </div>
                  <span className="text-[10px] flex-shrink-0" style={{ color: 'var(--text-faint)' }}>
                    {formatRelativeTime(entry.timestamp)}
                  </span>
                </div>
              ))}
            </div>
            <div className="px-4 py-2.5 flex items-center justify-between gap-2" style={{ borderTop: '1px solid var(--divider)' }}>
              <p className="text-[10px]" style={{ color: 'var(--text-faint)' }}>
                Most recent {recentAudit.length} {recentAudit.length === 1 ? 'event' : 'events'}
              </p>
              <AnimatePresence mode="wait">
                {confirmClearAudit ? (
                  <motion.div
                    key="confirm-clear"
                    initial={{ opacity: 0, scale: 0.9 }}
                    animate={{ opacity: 1, scale: 1 }}
                    exit={{ opacity: 0, scale: 0.9 }}
                    className="flex items-center gap-1.5"
                  >
                    <button
                      onClick={() => setConfirmClearAudit(false)}
                      className="text-xs px-2 py-1 rounded-lg transition-all"
                      style={{ color: 'var(--text-muted)', background: 'var(--bg-subtle)' }}
                    >
                      Cancel
                    </button>
                    <button
                      onClick={handleClearAudit}
                      className="text-xs px-2 py-1 rounded-lg font-semibold transition-all"
                      style={{
                        color: '#f87171',
                        background: 'rgba(239,68,68,0.12)',
                        border: '1px solid rgba(239,68,68,0.2)',
                      }}
                    >
                      Confirm clear
                    </button>
                  </motion.div>
                ) : (
                  <motion.button
                    key="clear"
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                    onClick={() => setConfirmClearAudit(true)}
                    className="text-xs px-2 py-1 rounded-lg transition-all"
                    style={{
                      color: 'var(--text-muted)',
                      background: 'var(--bg-subtle)',
                      border: '1px solid var(--border-subtle)',
                    }}
                  >
                    Clear log
                  </motion.button>
                )}
              </AnimatePresence>
            </div>
          </>
        )}
      </Section>

      {/* ── Diagnostics (v0.3.8 Phase E) ── */}
      <DiagnosticsSection />

      {/* ── Developer tools (v0.3.9) ── */}
      <DeveloperSection />

      {/* ── About ── */}
      <Section
        title="About"
        className="lg:col-span-2"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
        }
      >
        <Row>
          <div className="flex items-center justify-between gap-3">
            <div>
              <p className="text-sm font-semibold" style={{ color: 'var(--text-primary)' }}>MultiMouse</p>
              <p className="text-[11px] mt-0.5" style={{ color: 'var(--text-faint)' }}>
                {appVersion ? `v${appVersion}` : 'v—'} · {status?.device_name ?? '—'}
              </p>
            </div>
            <button
              onClick={checkUpdate}
              disabled={checkingUpdate}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold transition-all active:scale-95 disabled:opacity-50 flex-shrink-0"
              style={{
                background: 'var(--accent-soft-bg)',
                border: '1px solid var(--accent-soft-br)',
                color: 'var(--accent-primary)',
              }}
            >
              {checkingUpdate ? (
                <>
                  <motion.svg
                    className="w-3.5 h-3.5"
                    animate={{ rotate: 360 }}
                    transition={{ repeat: Infinity, duration: 1, ease: 'linear' }}
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke="currentColor"
                    strokeWidth={2.5}
                  >
                    <path strokeLinecap="round" strokeLinejoin="round"
                      d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                  </motion.svg>
                  Checking
                </>
              ) : (
                'Check for update'
              )}
            </button>
          </div>

          <AnimatePresence>
            {updateMsg && (
              <motion.p
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: 'auto' }}
                exit={{ opacity: 0, height: 0 }}
                className="text-[11px] mt-2.5 leading-relaxed"
                style={{ color: updateOk ? 'var(--success)' : 'var(--accent-primary)' }}
              >
                {updateMsg}
              </motion.p>
            )}
          </AnimatePresence>
        </Row>
        <Row noDivider>
          <p className="text-[11px] leading-relaxed" style={{ color: 'var(--text-faint)' }}>
            Share mouse and keyboard seamlessly across Mac, Windows, and Linux. Move the cursor to the configured screen edge to switch between computers.
          </p>
        </Row>
      </Section>

      </div>
    </div>
  );
};
