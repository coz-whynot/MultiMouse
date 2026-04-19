import { useState, useEffect, useMemo } from 'react';
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

      {/* ── Keyboard shortcut ── */}
      <Section
        title="Release Hotkey"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 18h.01M8 21h8a2 2 0 002-2V5a2 2 0 00-2-2H8a2 2 0 00-2 2v14a2 2 0 002 2z" />
          </svg>
        }
      >
        <Row noDivider>
          <div className="flex items-center justify-between gap-4">
            <div>
              <p className="text-sm font-medium" style={{ color: 'var(--text-secondary)' }}>
                Return cursor to this machine
              </p>
              <p className="text-[11px] mt-0.5" style={{ color: 'var(--text-faint)' }}>
                Tap Ctrl twice quickly to release control
              </p>
            </div>
            <kbd
              className="px-3 py-1.5 rounded-xl text-xs font-mono font-bold flex-shrink-0"
              style={{
                background: 'var(--accent-soft-bg)',
                border: '1px solid var(--accent-soft-br)',
                color: 'var(--accent-primary)',
                boxShadow: '0 2px 6px rgba(99,102,241,0.15)',
              }}
            >
              Ctrl × 2
            </kbd>
          </div>
        </Row>
      </Section>

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
