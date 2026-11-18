import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'happy-dom',
    // The page and the API on one origin, as behind the app's own domain: no CORS preflight.
    environmentOptions: { happyDOM: { url: 'https://cards.example.com' } },
    setupFiles: ['test/setup.ts'],
  },
});
