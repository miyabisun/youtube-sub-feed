import { mount } from 'svelte';
import 'normalize.css';
import './global.sass';
import App from './App.svelte';

mount(App, { target: document.getElementById('app') });
