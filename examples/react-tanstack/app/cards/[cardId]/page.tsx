import { HydrationBoundary } from '@tanstack/react-query';
import { CardEditor } from '../../../src/components/CardEditor';
import { prefetchCard } from '../../../src/prefetch';
import { requestSession } from '../../session';

export const dynamic = 'force-dynamic';

export default async function CardPage({ params }: { params: Promise<{ cardId: string }> }) {
  const { cardId } = await params;
  return (
    <HydrationBoundary state={await prefetchCard(await requestSession(), cardId)}>
      <CardEditor cardId={cardId} />
    </HydrationBoundary>
  );
}
