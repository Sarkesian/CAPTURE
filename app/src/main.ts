import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import Overlay from './overlay/Overlay.svelte';
import { navigate } from './lib/store.svelte';

// One bundle, two windows: the full settings app and the in-game overlay (#overlay).
const target = document.getElementById('app')!;
if (location.hash === '#overlay') {
  document.documentElement.classList.add('overlay');
  mount(Overlay, { target });
} else {
  if (location.hash === '#library') navigate('library');
  mount(App, { target });
}
