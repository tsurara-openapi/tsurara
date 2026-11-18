'use client';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { isHttpError } from '@tsurara/client';
import { type FormEvent, useState } from 'react';
import { fieldErrors, updateCardOptions } from '../cards';
import { getCardQueryOptions } from '../generated/query/getCard';
import { useSession } from '../session';

/** The detail and edit form of a card: a typed 422 is shown next to the field. */
export function CardEditor({ cardId }: { cardId: string }) {
  const { api, scope } = useSession();
  const queryClient = useQueryClient();
  const card = useQuery(getCardQueryOptions(api, { path: { cardId } }));
  const update = useMutation(updateCardOptions(api, queryClient, scope));
  const [title, setTitle] = useState<string | undefined>(undefined);
  if (card.error && isHttpError(card.error.result, 404)) return <p role="alert">No such card.</p>;
  if (!card.data) return <p>Loading…</p>;
  const errors = fieldErrors(update.error);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    update.mutate({ path: { cardId }, body: { title: title ?? card.data.title } });
  };
  return (
    <form onSubmit={submit}>
      <h1>{card.data.title}</h1>
      <label>
        Title
        <input value={title ?? card.data.title} onChange={(e) => setTitle(e.target.value)} />
      </label>
      {errors?.title && <p role="alert">{errors.title}</p>}
      <button type="submit" disabled={update.isPending}>
        Save
      </button>
    </form>
  );
}
