'use client';

import { useQueryClient } from '@tanstack/react-query';
import type { Client, QueryScope } from '@tsurara/client';
import { createContext, type ReactNode, useContext, useMemo, useState } from 'react';
import { createApi, type Session, scopeOf } from './api';
import { forgetScope } from './cards';

type SessionState = {
  readonly session: Session;
  readonly api: Client;
  readonly scope: QueryScope;
  /** Signs in to another tenant and drops what the previous one cached. */
  readonly switchTo: (next: Session) => void;
};

const SessionContext = createContext<SessionState | undefined>(undefined);

export function SessionProvider({ initial, children }: { initial: Session; children: ReactNode }) {
  const queryClient = useQueryClient();
  const [session, setSession] = useState(initial);
  const state = useMemo<SessionState>(
    () => ({
      session,
      api: createApi(session),
      scope: scopeOf(session),
      switchTo: (next) => {
        forgetScope(queryClient, scopeOf(session));
        setSession(next);
      },
    }),
    [session, queryClient],
  );
  return <SessionContext.Provider value={state}>{children}</SessionContext.Provider>;
}

export function useSession(): SessionState {
  const state = useContext(SessionContext);
  if (!state) throw new Error('useSession outside SessionProvider');
  return state;
}
