// @vitest-environment node
import { InfiniteQueryObserver, MutationObserver } from '@tanstack/react-query';
import { ApiError } from '@tsurara/client';
import { HttpResponse, http } from 'msw';
import { describe, expect, test } from 'vitest';
import { createApi, makeQueryClient, scopeOf } from '../src/api';
import { cardListOptions, fieldErrors, forgetScope, updateCardOptions } from '../src/cards';
import {
  type UpdateCardMockResponse,
  updateCardHandler,
} from '../src/generated/mocks/operations/updateCard';
import { getCardQueryKey, getCardQueryOptions } from '../src/generated/query/getCard';
import { acmeOne, getCard, listCards, session } from './fixtures';
import { server } from './server';

const acme = session('acme');

describe('the search list', () => {
  test('reads page after page and stops at the last', async () => {
    const requests: string[] = [];
    server.use(listCards(requests));
    const options = cardListOptions(createApi(acme), 'Acme');
    const data = await makeQueryClient().fetchInfiniteQuery({ ...options, pages: 3 });
    expect(data.pages.flatMap((p) => p.items.map((c) => c.id))).toEqual(['a1', 'a2', 'a3']);
    const last = data.pages.at(-1);
    expect(last && options.getNextPageParam(last)).toBeUndefined();
    expect(requests).toEqual(['?q=Acme', '?q=Acme&cursor=1', '?q=Acme&cursor=2']);
  });
});

describe('editing a card', () => {
  /** Edits `a1`; `respond` answers the PATCH. Returns the title the cache held while the server worked. */
  const edit = async (respond: (body: object) => UpdateCardMockResponse) => {
    const queryClient = makeQueryClient();
    const api = createApi(acme);
    const key = getCardQueryKey({ path: { cardId: 'a1' } }, scopeOf(acme));
    let optimistic: string | undefined;
    server.use(
      getCard(),
      listCards(),
      updateCardHandler(async ({ request }) => {
        optimistic = queryClient.getQueryData(key)?.title;
        return respond((await request.json()) as object);
      }),
    );
    await queryClient.fetchQuery(getCardQueryOptions(api, { path: { cardId: 'a1' } }));
    const observer = new MutationObserver(
      queryClient,
      updateCardOptions(api, queryClient, scopeOf(acme)),
    );
    const outcome = await observer
      .mutate({ path: { cardId: 'a1' }, body: { title: 'Renamed' } })
      .catch((e: unknown) => e);
    return { optimistic, outcome, after: queryClient.getQueryData(key), observer };
  };

  test('shows the change at once and keeps the server answer', async () => {
    const { optimistic, after } = await edit((body) => ({
      status: 200,
      body: { ...acmeOne, ...body },
    }));
    expect(optimistic).toBe('Renamed');
    expect(after?.title).toBe('Renamed');
  });

  test('rolls back a refused change and shows the typed field errors', async () => {
    const { optimistic, outcome, after, observer } = await edit(() => ({
      status: 422,
      body: { fields: { title: 'Too short' } },
    }));
    expect(optimistic).toBe('Renamed');
    expect(after?.title).toBe('Acme one');
    expect(outcome).toBeInstanceOf(ApiError);
    expect(fieldErrors(observer.getCurrentResult().error)).toEqual({ title: 'Too short' });
  });

  test('refetches the lists of the tenant after a change', async () => {
    const requests: string[] = [];
    server.use(listCards(requests), getCard(), updateCardHandler({ status: 200, body: acmeOne }));
    const queryClient = makeQueryClient();
    const api = createApi(acme);
    const list = new InfiniteQueryObserver(queryClient, cardListOptions(api, ''));
    const stop = list.subscribe(() => {});
    await queryClient.fetchInfiniteQuery(cardListOptions(api, ''));
    const before = requests.length;
    await new MutationObserver(
      queryClient,
      updateCardOptions(api, queryClient, scopeOf(acme)),
    ).mutate({
      path: { cardId: 'a1' },
      body: { done: true },
    });
    expect(requests.length).toBeGreaterThan(before);
    stop();
  });
});

describe('tenants', () => {
  test('keep separate entries, and switching drops only the previous one', async () => {
    server.use(getCard());
    const queryClient = makeQueryClient();
    await queryClient.fetchQuery(getCardQueryOptions(createApi(acme), { path: { cardId: 'a1' } }));
    const globex = session('globex');
    await queryClient.fetchQuery(
      getCardQueryOptions(createApi(globex), { path: { cardId: 'g1' } }),
    );
    forgetScope(queryClient, scopeOf(acme));
    expect(
      queryClient.getQueryData(getCardQueryKey({ path: { cardId: 'a1' } }, scopeOf(acme))),
    ).toBeUndefined();
    expect(
      queryClient.getQueryData(getCardQueryKey({ path: { cardId: 'g1' } }, scopeOf(globex)))?.title,
    ).toBe('Globex one');
  });
});

describe('retries', () => {
  test('a 503 is retried twice and a documented 404 not at all', async () => {
    let unavailable = 0;
    server.use(
      http.get('*/cards/busy', () => {
        unavailable++;
        return HttpResponse.json({}, { status: 503 });
      }),
      getCard(),
    );
    const queryClient = makeQueryClient(() => {});
    queryClient.setDefaultOptions({
      queries: { ...queryClient.getDefaultOptions().queries, retryDelay: 0 },
    });
    const api = createApi(acme);
    await queryClient
      .fetchQuery(getCardQueryOptions(api, { path: { cardId: 'busy' } }))
      .catch(() => undefined);
    expect(unavailable).toBe(3);
    const missing = await queryClient
      .fetchQuery(getCardQueryOptions(api, { path: { cardId: 'nope' } }))
      .catch((e: unknown) => e);
    expect(missing).toBeInstanceOf(ApiError);
    expect((missing as ApiError).status).toBe(404);
  });
});
