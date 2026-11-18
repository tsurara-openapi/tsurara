import { createClient } from '@acme/openapi';

export const api = createClient({ baseUrl: process.env.API_URL ?? 'http://localhost:3000' });
