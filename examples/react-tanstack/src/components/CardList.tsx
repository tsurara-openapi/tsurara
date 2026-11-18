'use client';

import { useInfiniteQuery, useQueryClient } from '@tanstack/react-query';
import { isHttpError } from '@tsurara/client';
import { useState } from 'react';
import { cardListOptions } from '../cards';
import { getCardQueryOptions } from '../generated/query/getCard';
import { useSession } from '../session';

/** The search list with infinite scrolling; hovering a card prefetches it before the detail opens. */
export function CardList({ initialSearch = '' }: { initialSearch?: string }) {
  const { api } = useSession();
  const queryClient = useQueryClient();
  const [search, setSearch] = useState(initialSearch);
  const cards = useInfiniteQuery(cardListOptions(api, search));
  return (
    <section>
      <input aria-label="Search" value={search} onChange={(e) => setSearch(e.target.value)} />
      {cards.error && isHttpError(cards.error.result, 401) && <p role="alert">Sign in again.</p>}
      {cards.error?.kind === 'network' && (
        <button type="button" onClick={() => void cards.refetch()}>
          Offline. Retry
        </button>
      )}
      <ul>
        {cards.data?.pages.flatMap((page) =>
          page.items.map((card) => (
            <li
              key={card.id}
              onMouseEnter={() =>
                void queryClient.prefetchQuery(
                  getCardQueryOptions(api, { path: { cardId: card.id } }),
                )
              }
            >
              <a href={`/cards/${card.id}`}>{card.title}</a>
            </li>
          )),
        )}
      </ul>
      {cards.hasNextPage && (
        <button
          type="button"
          disabled={cards.isFetchingNextPage}
          onClick={() => void cards.fetchNextPage()}
        >
          More
        </button>
      )}
    </section>
  );
}
