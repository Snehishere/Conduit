import { useState, useEffect, useCallback, useRef } from 'react';
import { SearchResult } from '../components/ui/SearchBar';
import { useDevices } from './useDevices';
import { useWebSocket } from './useWebSocket';
import type { ActiveView } from '../contexts/NavigationContext';

export function useSearch(setActiveView: (view: ActiveView) => void) {
  const [searchQuery, setSearchQuery] = useState('');
  const [searchFocused, setSearchFocused] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const { devices, setSelectedDevice } = useDevices();
  const { notifications } = useWebSocket();

  const buildSearchResults = useCallback((): SearchResult[] => {
    const q = searchQuery.toLowerCase();
    if (!q) return [];
    const results: SearchResult[] = [];

    devices.forEach((d) => {
      if (d.name.toLowerCase().includes(q) || d.device_type.toLowerCase().includes(q)) {
        results.push({
          id: `device-${d.id}`,
          title: d.name,
          subtitle: `${d.device_type} · ${d.os}`,
          category: 'device',
          onClick: () => { setActiveView('web'); setSelectedDevice(d.id); },
        });
      }
    });

    notifications.filter((n) => !n.dismissed).forEach((n) => {
      if (n.title.toLowerCase().includes(q) || n.body.toLowerCase().includes(q)) {
        results.push({
          id: `notif-${n.id}`,
          title: n.title,
          subtitle: n.body.slice(0, 60),
          category: 'notification',
          onClick: () => { setActiveView('notifications'); },
        });
      }
    });

    return results;
  }, [searchQuery, devices, notifications, setSelectedDevice, setActiveView]);

  const [searchResults, setSearchResults] = useState<SearchResult[]>([]);

  useEffect(() => {
    setSearchResults(buildSearchResults());
  }, [searchQuery, buildSearchResults]);

  useEffect(() => {
    if (searchFocused && searchRef.current) {
      searchRef.current.focus();
    }
  }, [searchFocused]);

  return {
    searchQuery,
    setSearchQuery,
    searchFocused,
    setSearchFocused,
    searchRef,
    searchResults,
  };
}
