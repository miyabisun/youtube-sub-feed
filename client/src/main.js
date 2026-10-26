import { mount } from 'svelte'
import 'normalize.css'
import './global.sass'
import App from './App.svelte'
import { followThemeColor } from './lib/theme-color.js'

followThemeColor()
mount(App, { target: document.getElementById('app') })
