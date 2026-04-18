import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { useStore } from '../store/useStore';
import { Settings, KnownDevice } from '../types';
import { LayoutEditor } from '../components/LayoutEditor';

interface ToggleProps {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}

const Toggle = ({ label, description, checked, onChange }: ToggleProps) => (
  <div className="flex items-center justify-between">
    <div>
      <p className="text-sm text-white/80">{label}</p>
      {description && <p className="text-xs text-white/30 mt-0.5">{description}</p>}
    </div>
    <button
      onClick={() => onChange(!checked)}
      className={`relative rounded-full transition-all duration-200 ${
        checked ? 'bg-accent-500' : 'bg-white/10'
      }`}
      style={{ height: '22px', width: '40px', flexShrink: 0 }}
    >
      <motion.span
        animate={{ x: checked ? 18 : 2 }}
        transition={{ type: 'spring', stiffness: 500, damping: 30 }}
        className="absolute top-0.5 w-4 h-4 rounded-full bg-white shadow"
        style={{ display: 'block' }}
      />
    </button>
  </div>
);

export const SettingsPage = () => {
  const { settings, setSettings, setPage } = useStore();
  const [knownDevices, setKnownDevices] = useState<KnownDevice[]>([]);
  const [relayInput, setRelayInput] = useState('');

  useEffect(() => {
    if (settings) setRelayInput(settings.relay_url ?? '');
    invoke<KnownDevice[]>('get_known_devices')
      .then(setKnownDevices)
      .catch(() => {});
  }, [settings?.relay_url]);

  if (!settings) return null;

  const update = async (patch: Partial<Settings>) => {
    const next = { ...settings, ...patch };
    setSettings(next);
    await invoke('update_settings', { settings: next });
  };

  const handleForget = async (id: string) => {
    await invoke('forget_device', { deviceId: id });
    setKnownDevices((prev) => prev.filter((d) => d.id !== id));
  };

  const handleRelayBlur = () => {
    update({ relay_url: relayInput.trim() });
  };

  return (
    <div className="flex flex-col flex-1 overflow-hidden">
      <div className="flex items-center gap-3 px-4 py-3 border-b border-white/[0.05]">
        <button
          onClick={() => setPage('home')}
          className="w-7 h-7 rounded-lg flex items-center justify-center
            text-white/40 hover:text-white hover:bg-white/5 transition-all"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
          </svg>
        </button>
        <span className="text-sm font-semibold text-white">Settings</span>
      </div>

      <div className="flex-1 overflow-y-auto px-4 py-4 space-y-6">

        {/* Layout editor */}
        <section>
          <p className="text-xs font-medium text-white/40 uppercase tracking-wider mb-3">
            Screen Layout
          </p>
          <LayoutEditor
            activeEdge={settings.transition_edge as any}
            onChange={(edge) => update({ transition_edge: edge })}
          />
          <p className="text-xs text-white/25 mt-2">
            Move cursor to the highlighted edge to switch to the connected device.
          </p>
        </section>

        {/* Behaviour */}
        <section>
          <p className="text-xs font-medium text-white/40 uppercase tracking-wider mb-3">
            Behaviour
          </p>
          <div className="space-y-4">
            <Toggle
              label="Launch on startup"
              description="Start MultiMouse when you log in"
              checked={settings.launch_on_startup}
              onChange={(v) => update({ launch_on_startup: v })}
            />
          </div>
        </section>

        {/* Hotkey */}
        <section>
          <p className="text-xs font-medium text-white/40 uppercase tracking-wider mb-3">
            Release Hotkey
          </p>
          <div className="bg-white/[0.04] border border-white/[0.07] rounded-xl px-4 py-3
            text-sm font-mono text-white/60">
            Double Ctrl
          </div>
          <p className="text-xs text-white/25 mt-2">
            Quickly press Ctrl twice to bring the cursor back to this machine.
          </p>
        </section>

        {/* Internet relay */}
        <section>
          <p className="text-xs font-medium text-white/40 uppercase tracking-wider mb-3">
            Internet Relay
          </p>
          <input
            type="text"
            value={relayInput}
            onChange={(e) => setRelayInput(e.target.value)}
            onBlur={handleRelayBlur}
            placeholder="relay.example.com:57173"
            className="w-full bg-white/[0.05] border border-white/[0.09] rounded-xl px-3 py-2.5
              text-xs text-white/70 placeholder:text-white/20
              focus:outline-none focus:border-accent-500/40 font-mono"
          />
          <p className="text-xs text-white/25 mt-2">
            Run <span className="font-mono text-white/40">./relay</span> on any server and paste its address here to enable internet connections.
          </p>
        </section>

        {/* File transfer */}
        <section>
          <p className="text-xs font-medium text-white/40 uppercase tracking-wider mb-3">
            File Transfer
          </p>
          <div className="bg-white/[0.03] border border-white/[0.06] rounded-xl px-4 py-3">
            <p className="text-xs text-white/50">
              Drop files onto this window while connected to send them to the other device.
              Files land in the recipient's Downloads folder.
            </p>
          </div>
        </section>

        {/* Paired devices */}
        {knownDevices.length > 0 && (
          <section>
            <p className="text-xs font-medium text-white/40 uppercase tracking-wider mb-3">
              Paired Devices
            </p>
            <div className="space-y-2">
              {knownDevices.map((d) => (
                <div
                  key={d.id}
                  className="flex items-center justify-between px-3 py-2.5
                    bg-white/[0.03] border border-white/[0.06] rounded-xl"
                >
                  <div className="flex items-center gap-2.5 min-w-0">
                    <div className="w-7 h-7 rounded-lg bg-white/[0.06] flex items-center justify-center flex-shrink-0">
                      <svg className="w-3.5 h-3.5 text-white/40" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                          d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
                      </svg>
                    </div>
                    <div className="min-w-0">
                      <p className="text-xs font-medium text-white/70 truncate">{d.name}</p>
                      <p className="text-[10px] text-white/30 truncate">{d.addr}</p>
                    </div>
                  </div>
                  <button
                    onClick={() => handleForget(d.id)}
                    className="text-white/25 hover:text-red-400 transition-colors flex-shrink-0 ml-2 p-1"
                    title="Forget device"
                  >
                    <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                    </svg>
                  </button>
                </div>
              ))}
            </div>
            <p className="text-xs text-white/25 mt-2">
              Paired devices reconnect automatically — no PIN required.
            </p>
          </section>
        )}

      </div>
    </div>
  );
};
