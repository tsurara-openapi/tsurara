// A bundled browser app: it imports from the barrel, and only the operations it uses end up in the
// bundle (`sideEffects: false` in the shared package).
import { createClient, getPet, isHttpError, listPets } from '@acme/openapi';

const api = createClient({ baseUrl: 'http://localhost:3000' });

const list = document.querySelector('#pets');
const r = await listPets(api, { query: { limit: 20 } });
if (r.ok && list) {
  for (const pet of r.data) {
    const item = document.createElement('li');
    item.textContent = pet.name;
    list.append(item);
  }
}
const first = await getPet(api, { path: { petId: 1 } });
if (isHttpError(first, 404)) console.warn(first.error.title);
