import { HydrationBoundary } from '@tanstack/react-query';
import { CardList } from '../src/components/CardList';
import { prefetchCardList } from '../src/prefetch';
import { requestSession } from './session';

// Rendered per request: the list depends on who is signed in.
export const dynamic = 'force-dynamic';

export default async function Home() {
  return (
    <HydrationBoundary state={await prefetchCardList(await requestSession())}>
      <CardList />
    </HydrationBoundary>
  );
}
