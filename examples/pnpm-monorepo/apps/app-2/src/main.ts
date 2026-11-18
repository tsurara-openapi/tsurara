import { listPets, unwrap } from '@acme/openapi';
import { createApi } from './api.ts';

const api = createApi(process.env.API_URL ?? 'http://localhost:3000');
const pets = unwrap(await listPets(api, { query: { limit: 10 }, context: { requestId: 'cli' } }));
for (const pet of pets) console.log(`${pet.id}\t${pet.name}`);
