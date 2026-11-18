'use client';

import { QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import type { Session } from '../src/api';
import { getQueryClient } from '../src/query-client';
import { SessionProvider } from '../src/session';

export function Providers({ session, children }: { session: Session; children: ReactNode }) {
  return (
    <QueryClientProvider client={getQueryClient()}>
      <SessionProvider initial={session}>{children}</SessionProvider>
    </QueryClientProvider>
  );
}
