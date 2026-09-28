import { useNavigation, ActiveView } from '../contexts/NavigationContext';
import { useSearch } from './useSearch';

export type { ActiveView };

/**
 * Composes navigation context with search functionality into a single hook.
 * Reduces AppInner from two separate hook calls to one for all navigation-related state.
 */
export function useAppNavigation() {
  const {
    activeView, setActiveView,
    selectedDevice, setSelectedDevice,
    screenMirrorDevice, setScreenMirrorDevice,
    remoteInputDevice, setRemoteInputDevice,
  } = useNavigation();

  const {
    searchQuery, setSearchQuery,
    searchFocused, setSearchFocused,
    searchRef, searchResults,
  } = useSearch(setActiveView);

  return {
    activeView,
    setActiveView,
    selectedDevice,
    setSelectedDevice,
    screenMirrorDevice,
    setScreenMirrorDevice,
    remoteInputDevice,
    setRemoteInputDevice,
    searchQuery,
    setSearchQuery,
    searchFocused,
    setSearchFocused,
    searchRef,
    searchResults,
  };
}
