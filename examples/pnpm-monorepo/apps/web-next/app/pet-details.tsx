'use client';

// A client component: the same functions run in the browser.
import { createClient, getPet, isHttpError, type Pet } from '@acme/openapi';
import { useEffect, useState } from 'react';

const api = createClient({ baseUrl: 'http://localhost:3000' });

export function PetDetails({ petId }: { petId: number }) {
  const [pet, setPet] = useState<Pet | string>('loading');
  useEffect(() => {
    void getPet(api, { path: { petId } }).then((r) =>
      setPet(r.ok ? r.data : isHttpError(r, 404) ? r.error.title : r.kind),
    );
  }, [petId]);
  return <p>{typeof pet === 'string' ? pet : pet.name}</p>;
}
