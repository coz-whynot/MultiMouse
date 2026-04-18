import { getCurrentWindow } from '@tauri-apps/api/window';

interface Props {
  onSettings: () => void;
}

export const TitleBar = ({ onSettings }: Props) => {
  const close = () => getCurrentWindow().hide();

  return (
    <div
      data-tauri-drag-region
      className="flex items-center justify-between px-4 h-12 flex-shrink-0"
    >
      <div className="flex items-center gap-2" data-tauri-drag-region>
        <div className="w-7 h-7 rounded-lg bg-gradient-to-br from-accent-500 to-purple-500
          flex items-center justify-center shadow-lg shadow-accent-500/20">
          <svg className="w-4 h-4 text-white" viewBox="0 0 24 24" fill="currentColor">
            <path d="M13 6a3 3 0 11-6 0 3 3 0 016 0zM18 8a2 2 0 11-4 0 2 2 0 014 0zM14 15a4 4 0 00-8 0v1h8v-1zM6 8a2 2 0 11-4 0 2 2 0 014 0zM16 18v-1a5.97 5.97 0 00-.75-2.906A3.005 3.005 0 0119 15v1h-3zM4.75 14.094A5.97 5.97 0 004 17v1H1v-1a3 3 0 013.75-2.906z" />
          </svg>
        </div>
        <span className="text-sm font-semibold text-white">MultiMouse</span>
      </div>

      <div className="flex items-center gap-1">
        <button
          onClick={onSettings}
          className="w-7 h-7 rounded-lg flex items-center justify-center
            text-white/40 hover:text-white/80 hover:bg-white/5 transition-all"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8}
              d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
          </svg>
        </button>
        <button
          onClick={close}
          className="w-7 h-7 rounded-lg flex items-center justify-center
            text-white/40 hover:text-white/80 hover:bg-white/5 transition-all"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
    </div>
  );
};
