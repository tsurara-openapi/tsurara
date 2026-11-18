// @vitest-environment node
import { HydrationBoundary, QueryClientProvider } from '@tanstack/react-query';
import { renderToString } from 'react-dom/server';
import { expect, test } from 'vitest';
import { CardList } from '../src/components/CardList';
import { prefetchCardList } from '../src/prefetch';
import { getQueryClient } from '../src/query-client';
import { SessionProvider } from '../src/session';
import { listCards, session } from './fixtures';
import { server } from './server';

test('the server makes a QueryClient per request', () => {
  expect(getQueryClient()).not.toBe(getQueryClient());
});

test('two concurrent requests of different tenants do not share data', async () => {
  server.use(listCards());
  const render = async (tenant: string) => {
    const state = await prefetchCardList(session(tenant));
    // What the browser renders first, from the dehydrated state alone.
    return renderToString(
      <QueryClientProvider client={getQueryClient()}>
        <SessionProvider initial={session(tenant)}>
          <HydrationBoundary state={state}>
            <CardList />
          </HydrationBoundary>
        </SessionProvider>
      </QueryClientProvider>,
    );
  };
  const [acme, globex] = await Promise.all([render('acme'), render('globex')]);
  expect(acme).toContain('Acme one');
  expect(acme).not.toContain('Globex');
  expect(globex).toContain('Globex one');
  expect(globex).not.toContain('Acme');
});
