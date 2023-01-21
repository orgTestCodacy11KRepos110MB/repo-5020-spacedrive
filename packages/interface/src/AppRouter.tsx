import { lazy } from '@loadable/component';
import { useCurrentLibrary, useInvalidateQuery } from '@sd/client';
import { Navigate, Route, Routes } from 'react-router-dom';
import { useKeybindHandler } from '~/hooks/useKeyboardHandler';

import { AppLayout } from './AppLayout';
import OnboardingRoot, { ONBOARDING_SCREENS } from './components/onboarding/OnboardingRoot';
import MediaScreen from './screens/Media';
import PeopleScreen from './screens/People';
import SpacesScreen from './screens/Spaces';

// Using React.lazy breaks hot reload so we don't use it.
const DebugScreen = lazy(() => import('./screens/Debug'));
const SettingsScreen = lazy(() => import('./screens/settings/Settings'));
const SettingsSubPageScreen = lazy(() => import('./screens/settings/SettingsSubPage'));
const TagExplorer = lazy(() => import('./screens/TagExplorer'));
const PhotosScreen = lazy(() => import('./screens/Media'));
const OverviewScreen = lazy(() => import('./screens/Overview'));
const ContentScreen = lazy(() => import('./screens/People'));
const LocationExplorer = lazy(() => import('./screens/LocationExplorer'));
const OnboardingScreen = lazy(() => import('./components/onboarding/OnboardingStart'));
const NotFound = lazy(() => import('./NotFound'));

const AppearanceSettings = lazy(() => import('./screens/settings/client/AppearanceSettings'));
const ExtensionSettings = lazy(() => import('./screens/settings/client/ExtensionsSettings'));
const GeneralSettings = lazy(() => import('./screens/settings/client/GeneralSettings'));
const KeybindingSettings = lazy(() => import('./screens/settings/client/KeybindingSettings'));
const PrivacySettings = lazy(() => import('./screens/settings/client/PrivacySettings'));
const AboutSpacedrive = lazy(() => import('./screens/settings/info/AboutSpacedrive'));
const Changelog = lazy(() => import('./screens/settings/info/Changelog'));
const Support = lazy(() => import('./screens/settings/info/Support'));
const ContactsSettings = lazy(() => import('./screens/settings/library/ContactsSettings'));
const KeysSettings = lazy(() => import('./screens/settings/library/KeysSetting'));
const LibraryGeneralSettings = lazy(
	() => import('./screens/settings/library/LibraryGeneralSettings')
);
const LocationsSettings = lazy(() => import('./screens/settings/library/LocationsSettings'));
const LocationSettings = lazy(() => import('./screens/settings/library/location/LocationSettings'));
const NodesSettings = lazy(() => import('./screens/settings/library/NodesSettings'));
const SecuritySettings = lazy(() => import('./screens/settings/library/SecuritySettings'));
const SharingSettings = lazy(() => import('./screens/settings/library/SharingSettings'));
const SyncSettings = lazy(() => import('./screens/settings/library/SyncSettings'));
const TagsSettings = lazy(() => import('./screens/settings/library/TagsSettings'));
const ExperimentalSettings = lazy(() => import('./screens/settings/node/ExperimentalSettings'));
const LibrarySettings = lazy(() => import('./screens/settings/node/LibrariesSettings'));
const P2PSettings = lazy(() => import('./screens/settings/node/P2PSettings'));

export function AppRouter() {
	const { library } = useCurrentLibrary();

	useKeybindHandler();
	useInvalidateQuery();

	return (
		<Routes>
			<Route path="onboarding" element={<OnboardingRoot />}>
				<Route index element={<Navigate to="start" />} />
				{ONBOARDING_SCREENS.map(({ key, component: ScreenComponent }, index) => (
					<Route key={key} path={key} element={<ScreenComponent />} />
				))}
			</Route>
			<Route element={<AppLayout />}>
				{/* As we are caching the libraries in localStore so this *shouldn't* result is visual problems unless something else is wrong */}
				{library === undefined ? (
					<Route
						path="*"
						element={
							<h1 className="p-4 text-white">Please select or create a library in the sidebar.</h1>
						}
					/>
				) : (
					<>
						<Route index element={<Navigate to="/overview" />} />
						<Route path="overview" element={<OverviewScreen />} />
						<Route path="spaces" element={<SpacesScreen />} />
						<Route path="people" element={<PeopleScreen />} />
						<Route path="media" element={<MediaScreen />} />
						<Route path="debug" element={<DebugScreen />} />
						<Route path="settings" element={<SettingsScreen />}>
							<Route index element={<GeneralSettings />} />
							<Route path="general" element={<GeneralSettings />} />
							<Route path="appearance" element={<AppearanceSettings />} />
							<Route path="keybindings" element={<KeybindingSettings />} />
							<Route path="extensions" element={<ExtensionSettings />} />
							<Route path="p2p" element={<P2PSettings />} />
							<Route path="contacts" element={<ContactsSettings />} />
							<Route path="experimental" element={<ExperimentalSettings />} />
							<Route path="keys" element={<KeysSettings />} />
							<Route path="libraries" element={<LibrarySettings />} />
							<Route path="security" element={<SecuritySettings />} />
							<Route path="locations" element={<LocationSettings />} />
							<Route path="sharing" element={<SharingSettings />} />
							<Route path="sync" element={<SyncSettings />} />
							<Route path="tags" element={<TagsSettings />} />
							<Route path="library" element={<LibraryGeneralSettings />} />
							<Route path="locations" element={<SettingsSubPageScreen />}>
								<Route index element={<LocationsSettings />} />
								<Route path="location" element={<LocationSettings />} />
							</Route>
							<Route path="tags" element={<TagsSettings />} />
							<Route path="nodes" element={<NodesSettings />} />
							<Route path="keys" element={<KeysSettings />} />
							<Route path="privacy" element={<PrivacySettings />} />
							<Route path="about" element={<AboutSpacedrive />} />
							<Route path="changelog" element={<Changelog />} />
							<Route path="support" element={<Support />} />
						</Route>
						<Route path="location/:id" element={<LocationExplorer />} />
						<Route path="tag/:id" element={<TagExplorer />} />
						<Route path="*" element={<NotFound />} />
					</>
				)}
			</Route>
		</Routes>
	);
}
