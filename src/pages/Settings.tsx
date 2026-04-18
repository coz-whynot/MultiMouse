import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { useStore } from '../store/useStore';
import { Settings, KnownDevice } from '../types';

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
      <p className="text-sm font-medium" style={{ color: 'rgba(255,255,255,0.78)' }}>{label}</p>
      {description && (
        <p className="text-[11px] mt-0.5 leading-relaxed" style={{ color: 'rgba(255,255,255,0.28)' }}>
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
          : 'rgba(255,255,255,0.1)',
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
}: {
  title: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
}) => (
  <div className="space-y-2">
    <div className="flex items-center gap-2 px-1">
      {icon && <span style={{ color: 'rgba(255,255,255,0.28)' }}>{icon}</span>}
      <p
        className="text-[10px] font-bold uppercase tracking-widest"
        style={{ color: 'rgba(255,255,255,0.28)' }}
      >
        {title}
      </p>
    </div>
    <div
      className="rounded-2xl overflow-hidden"
      style={{
        background: 'rgba(255,255,255,0.035)',
        border: '1px solid rgba(255,255,255,0.07)',
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
      borderBottom: noDivider ? 'none' : '1px solid rgba(255,255,255,0.05)',
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
    <div className="px-4 py-3 flex items-center gap-3" style={{ borderBottom: '1px solid rgba(255,255,255,0.05)' }}>
      {/* Avatar */}
      <div
        className="w-8 h-8 rounded-xl flex items-center justify-center flex-shrink-0 text-sm font-bold"
        style={{
          background: 'rgba(99,102,241,0.12)',
          border: '1px solid rgba(99,102,241,0.2)',
          color: '#a78bfa',
        }}
      >
        {device.name.charAt(0).toUpperCase()}
      </div>

      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium truncate" style={{ color: 'rgba(255,255,255,0.75)' }}>
          {device.name}
        </p>
        <p className="text-[10px] font-mono truncate" style={{ color: 'rgba(255,255,255,0.25)' }}>
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
                style={{ color: 'rgba(255,255,255,0.35)', background: 'rgba(255,255,255,0.06)' }}
              >
                Keep
              </button>
              <button
                onClick={() => onForget(device.id)}
                className="text-xs px-2 py-1 rounded-lg transition-all font-semibold"
                style={{ color: '#f87171', background: 'rgba(239,68,68,0.12)', border: '1px solid rgba(239,68,68,0.2)' }}
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
              style={{ color: 'rgba(255,255,255,0.2)', background: 'rgba(255,255,255,0.04)' }}
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

  useEffect(() => {
    if (settings) setRelayInput(settings.relay_url ?? '');
    invoke<KnownDevice[]>('get_known_devices').then(setKnownDevices).catch(() => {});
  }, [settings?.relay_url]);

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
    <div className="flex flex-col flex-1 overflow-y-auto px-3 py-3 gap-4 pb-2">

      {/* ── General ── */}
      {/* TODO: full light-theme requires per-component refactor */}
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
        <Row noDivider>
          <div className="flex items-center justify-between gap-4">
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium" style={{ color: 'rgba(255,255,255,0.78)' }}>Theme</p>
              <p className="text-[11px] mt-0.5 leading-relaxed" style={{ color: 'rgba(255,255,255,0.28)' }}>
                Light mode is limited in this version
              </p>
            </div>
            <div
              className="flex items-center rounded-xl p-0.5 flex-shrink-0"
              style={{
                background: 'rgba(255,255,255,0.05)',
                border: '1px solid rgba(255,255,255,0.08)',
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
                      color: active ? 'white' : 'rgba(255,255,255,0.42)',
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

      {/* ── Edge switching ── */}
      <Section
        title="Edge Switching"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M13 5l7 7-7 7M5 5l7 7-7 7" />
          </svg>
        }
      >
        <Row noDivider>
          <div className="flex items-start justify-between gap-4 mb-3">
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium" style={{ color: 'rgba(255,255,255,0.78)' }}>
                Edge dwell time
              </p>
              <p className="text-[11px] mt-0.5 leading-relaxed" style={{ color: 'rgba(255,255,255,0.28)' }}>
                Time to hold cursor at edge before switching
              </p>
            </div>
            <span
              className="text-xs font-mono font-bold px-2.5 py-1 rounded-lg flex-shrink-0"
              style={{
                background: 'rgba(99,102,241,0.1)',
                border: '1px solid rgba(99,102,241,0.22)',
                color: '#a78bfa',
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
            style={{ accentColor: '#a78bfa' }}
          />
          <div className="flex justify-between mt-1 text-[10px]" style={{ color: 'rgba(255,255,255,0.25)' }}>
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
              <p className="text-sm font-medium" style={{ color: 'rgba(255,255,255,0.78)' }}>
                Return cursor to this machine
              </p>
              <p className="text-[11px] mt-0.5" style={{ color: 'rgba(255,255,255,0.28)' }}>
                Tap Ctrl twice quickly to release control
              </p>
            </div>
            <kbd
              className="px-3 py-1.5 rounded-xl text-xs font-mono font-bold flex-shrink-0"
              style={{
                background: 'rgba(99,102,241,0.1)',
                border: '1px solid rgba(99,102,241,0.22)',
                color: '#a78bfa',
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
          <p className="text-[11px] mb-2.5 leading-relaxed" style={{ color: 'rgba(255,255,255,0.35)' }}>
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
              background: 'rgba(255,255,255,0.05)',
              border: `1.5px solid ${relayInput ? 'rgba(99,102,241,0.3)' : 'rgba(255,255,255,0.08)'}`,
              color: 'rgba(255,255,255,0.65)',
            }}
          />
        </Row>
      </Section>

      {/* ── Paired Devices ── */}
      <Section
        title={`Paired Devices${knownDevices.length > 0 ? ` · ${knownDevices.length}` : ''}`}
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round"
              d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
          </svg>
        }
      >
        {knownDevices.length === 0 ? (
          <div className="px-4 py-5 flex flex-col items-center gap-1.5">
            <svg className="w-7 h-7 mb-1" fill="none" viewBox="0 0 24 24" stroke="rgba(255,255,255,0.15)" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round"
                d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
            </svg>
            <p className="text-xs" style={{ color: 'rgba(255,255,255,0.25)' }}>No paired devices yet</p>
            <p className="text-[10px] text-center" style={{ color: 'rgba(255,255,255,0.15)' }}>
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
              <p className="text-[10px]" style={{ color: 'rgba(255,255,255,0.2)' }}>
                Paired devices reconnect automatically without a PIN.
              </p>
            </div>
          </>
        )}
      </Section>

      {/* ── About ── */}
      <Section
        title="About"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
        }
      >
        <Row>
          <div className="flex items-center justify-between gap-3">
            <div>
              <p className="text-sm font-semibold text-white">MultiMouse</p>
              <p className="text-[11px] mt-0.5" style={{ color: 'rgba(255,255,255,0.3)' }}>
                v0.1.0 · {status?.device_name ?? '—'}
              </p>
            </div>
            <button
              onClick={checkUpdate}
              disabled={checkingUpdate}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold transition-all active:scale-95 disabled:opacity-50 flex-shrink-0"
              style={{
                background: 'rgba(99,102,241,0.1)',
                border: '1px solid rgba(99,102,241,0.22)',
                color: '#a78bfa',
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
                style={{ color: updateOk ? '#34d399' : '#a78bfa' }}
              >
                {updateMsg}
              </motion.p>
            )}
          </AnimatePresence>
        </Row>
        <Row noDivider>
          <p className="text-[11px] leading-relaxed" style={{ color: 'rgba(255,255,255,0.22)' }}>
            Share mouse and keyboard seamlessly across Mac, Windows, and Linux. Move the cursor to the configured screen edge to switch between computers.
          </p>
        </Row>
      </Section>

    </div>
  );
};
