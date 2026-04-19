import { create } from 'zustand';
import {
  PeerInfo,
  AppStatus,
  Settings,
  PairingRequest,
  TransferInfo,
  FileOffer,
  IncomingDeepLink,
} from '../types';

interface Store {
  peers: PeerInfo[];
  status: AppStatus | null;
  settings: Settings | null;
  pairingRequest: PairingRequest | null;
  connectingTo: string | null;
  shownPin: string | null;
  currentPage: 'home' | 'layout' | 'settings';
  transfers: TransferInfo[];
  fileOffer: FileOffer | null;
  incomingDeepLink: IncomingDeepLink | null;

  setPeers: (peers: PeerInfo[]) => void;
  setStatus: (status: AppStatus) => void;
  setSettings: (settings: Settings) => void;
  setPairingRequest: (req: PairingRequest | null) => void;
  setConnectingTo: (id: string | null) => void;
  setShownPin: (pin: string | null) => void;
  setPage: (page: 'home' | 'layout' | 'settings') => void;
  setTransfers: (transfers: TransferInfo[]) => void;
  setFileOffer: (offer: FileOffer | null) => void;
  setIncomingDeepLink: (link: IncomingDeepLink | null) => void;
}

export const useStore = create<Store>((set) => ({
  peers: [],
  status: null,
  settings: null,
  pairingRequest: null,
  connectingTo: null,
  shownPin: null,
  currentPage: 'home',
  transfers: [],
  fileOffer: null,
  incomingDeepLink: null,

  setPeers: (peers) => set({ peers }),
  setStatus: (status) => set({ status }),
  setSettings: (settings) => set({ settings }),
  setPairingRequest: (req) => set({ pairingRequest: req }),
  setConnectingTo: (id) => set({ connectingTo: id }),
  setShownPin: (pin) => set({ shownPin: pin }),
  setPage: (page) => set({ currentPage: page }),
  setTransfers: (transfers) => set({ transfers }),
  setFileOffer: (offer) => set({ fileOffer: offer }),
  setIncomingDeepLink: (link) => set({ incomingDeepLink: link }),
}));
