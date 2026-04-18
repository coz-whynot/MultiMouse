import { create } from 'zustand';
import { PeerInfo, AppStatus, Settings, PairingRequest, TransferInfo, FileOffer } from '../types';

interface Store {
  peers: PeerInfo[];
  status: AppStatus | null;
  settings: Settings | null;
  pairingRequest: PairingRequest | null;
  connectingTo: string | null;
  pinInput: string;
  currentPage: 'home' | 'settings' | 'pairing';
  transfers: TransferInfo[];
  fileOffer: FileOffer | null;

  setPeers: (peers: PeerInfo[]) => void;
  setStatus: (status: AppStatus) => void;
  setSettings: (settings: Settings) => void;
  setPairingRequest: (req: PairingRequest | null) => void;
  setConnectingTo: (id: string | null) => void;
  setPinInput: (pin: string) => void;
  setPage: (page: 'home' | 'settings' | 'pairing') => void;
  setTransfers: (transfers: TransferInfo[]) => void;
  setFileOffer: (offer: FileOffer | null) => void;
}

export const useStore = create<Store>((set) => ({
  peers: [],
  status: null,
  settings: null,
  pairingRequest: null,
  connectingTo: null,
  pinInput: '',
  currentPage: 'home',
  transfers: [],
  fileOffer: null,

  setPeers: (peers) => set({ peers }),
  setStatus: (status) => set({ status }),
  setSettings: (settings) => set({ settings }),
  setPairingRequest: (req) => set({ pairingRequest: req }),
  setConnectingTo: (id) => set({ connectingTo: id }),
  setPinInput: (pin) => set({ pinInput: pin }),
  setPage: (page) => set({ currentPage: page }),
  setTransfers: (transfers) => set({ transfers }),
  setFileOffer: (offer) => set({ fileOffer: offer }),
}));
