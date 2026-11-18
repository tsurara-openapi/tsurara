import { type DehydratedState, dehydrate } from '@tanstack/react-query';
import { createApi, type Session } from './api';
import { cardListOptions } from './cards';
import { getCardQueryOptions } from './generated/query/getCard';
import { getQueryClient } from './query-client';

/** The first page of the list, fetched on the server for a request and handed to the browser. */
export async function prefetchCardList(session: Session, search = ''): Promise<DehydratedState> {
  const queryClient = getQueryClient();
  await queryClient.prefetchInfiniteQuery(cardListOptions(createApi(session), search));
  return dehydrate(queryClient);
}

/** A card, fetched on the server for its page. */
export async function prefetchCard(session: Session, cardId: string): Promise<DehydratedState> {
  const queryClient = getQueryClient();
  await queryClient.prefetchQuery(getCardQueryOptions(createApi(session), { path: { cardId } }));
  return dehydrate(queryClient);
}
