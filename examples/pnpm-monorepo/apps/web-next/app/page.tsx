// A server component: the generated functions run on the server at request time.
import { listPets } from '@acme/openapi';
import { api } from './api.ts';
import { PetDetails } from './pet-details.tsx';

export const dynamic = 'force-dynamic';

export default async function Page() {
  const r = await listPets(api, { query: { limit: 20 } });
  return (
    <main>
      <ul>{r.ok ? r.data.map((p) => <li key={p.id}>{p.name}</li>) : <li>{r.kind}</li>}</ul>
      <PetDetails petId={1} />
    </main>
  );
}
