import { svelte } from '@sveltejs/vite-plugin-svelte';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';
import path from 'path';

export default defineConfig({
	plugins: [svelte({ preprocess: vitePreprocess() })],
	resolve: {
		alias: {
			$lib: path.resolve('./src/lib')
		}
	},
	base: './',
	build: {
		outDir: 'build'
	},
	server: {
		host: '0.0.0.0',
		port: 5173,
		proxy: {
			'/api': 'http://localhost:3000'
		}
	}
});
