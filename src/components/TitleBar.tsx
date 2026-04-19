import { invoke } from '@tauri-apps/api/core';

export const TitleBar = () => {
  // Use the Rust `hide_window` command so the activation-policy flip (macOS
  // Regular → Accessory) happens in sync with the window hiding. Calling
  // getCurrentWindow().hide() directly leaves the app showing in the dock.
  const close = () => invoke('hide_window').catch(() => {});

  return (
    <div
      data-tauri-drag-region
      className="flex items-center justify-between px-4 h-12 flex-shrink-0"
    >
      <div className="flex items-center gap-2.5" data-tauri-drag-region>
        <div
          className="w-8 h-8 rounded-xl flex items-center justify-center"
          style={{
            background: 'linear-gradient(135deg, #6366f1 0%, #a855f7 100%)',
            boxShadow: '0 4px 12px rgba(99,102,241,0.5)',
          }}
        >
          <svg className="w-4 h-4 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
              d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
          </svg>
        </div>
        <span
          className="text-sm font-bold tracking-tight"
          style={{ color: 'var(--text-primary)' }}
        >
          MultiMouse
        </span>
      </div>

      <div className="flex items-center gap-0.5">
        <button
          onClick={close}
          className="w-8 h-8 rounded-xl flex items-center justify-center transition-all"
          style={{ color: 'var(--text-muted)' }}
          onMouseEnter={(e) => {
            (e.currentTarget as HTMLButtonElement).style.color = '#ef4444';
            (e.currentTarget as HTMLButtonElement).style.background = 'rgba(239,68,68,0.12)';
          }}
          onMouseLeave={(e) => {
            (e.currentTarget as HTMLButtonElement).style.color = '';
            // Clear inline color so it falls back to the var(--text-muted) stylesheet value.
            (e.currentTarget as HTMLButtonElement).style.background = 'transparent';
            (e.currentTarget as HTMLButtonElement).style.color = 'var(--text-muted)';
          }}
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
    </div>
  );
};
