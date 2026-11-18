import type { QueryClient } from '@tanstack/react-query';
import {
  type ApiError,
  type Client,
  isHttpError,
  type QueryScope,
  queryScopeFilter,
} from '@tsurara/client';
import type { UpdateCardData, UpdateCardResult } from './generated/operations/updateCard';
import { getCardQueryKey } from './generated/query/getCard';
import { listCardsInfiniteQueryOptions, listCardsQueryFilter } from './generated/query/listCards';
import { updateCardMutationOptions } from './generated/query/updateCard';
import type { Card } from './generated/schemas/Card';

/** The search list, a page at a time (the pagination is `x-tsurara-pagination` in openapi.yaml). */
export const cardListOptions = (api: Client, search: string) =>
  listCardsInfiniteQueryOptions(api, search === '' ? {} : { query: { q: search } });

/**
 * Editing a card: the change shows at once (optimistic update), is rolled back when the server
 * refuses it, and on success the card's entry is replaced and every list of the tenant is refetched.
 */
export const updateCardOptions = (api: Client, queryClient: QueryClient, scope: QueryScope) => ({
  ...updateCardMutationOptions(api),
  onMutate: async (input: UpdateCardData) => {
    const key = getCardQueryKey({ path: input.path }, scope);
    await queryClient.cancelQueries({ queryKey: key });
    const previous = queryClient.getQueryData(key);
    if (previous) queryClient.setQueryData(key, { ...previous, ...input.body });
    return { previous };
  },
  onError: (
    _error: ApiError<UpdateCardResult>,
    input: UpdateCardData,
    context: { previous: Card | undefined } | undefined,
  ) => {
    if (context?.previous)
      queryClient.setQueryData(getCardQueryKey({ path: input.path }, scope), context.previous);
  },
  onSuccess: async (card: Card) => {
    queryClient.setQueryData(getCardQueryKey({ path: { cardId: card.id } }, scope), card);
    await queryClient.invalidateQueries(listCardsQueryFilter(scope));
  },
});

/** The field errors of a refused edit (the documented 422 body), if that is why it failed. */
export const fieldErrors = (
  error: ApiError<UpdateCardResult> | null,
): Record<string, string> | undefined =>
  error && isHttpError(error.result, 422) ? error.result.error.fields : undefined;

/** Signing out or switching tenants: drop everything the tenant cached. */
export const forgetScope = (queryClient: QueryClient, scope: QueryScope): void => {
  queryClient.removeQueries(queryScopeFilter(scope));
};
