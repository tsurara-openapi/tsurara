import { QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, expect, test } from 'vitest';
import { makeQueryClient } from '../src/api';
import { CardEditor } from '../src/components/CardEditor';
import { CardList } from '../src/components/CardList';
import { updateCardHandler } from '../src/generated/mocks/operations/updateCard';
import { SessionProvider } from '../src/session';
import { getCard, listCards, session } from './fixtures';
import { server } from './server';

afterEach(cleanup);

const app = (children: ReactNode) =>
  render(
    <QueryClientProvider client={makeQueryClient()}>
      <SessionProvider initial={session('acme')}>{children}</SessionProvider>
    </QueryClientProvider>,
  );

test('the list loads more pages on demand', async () => {
  server.use(listCards());
  app(<CardList />);
  expect(await screen.findByText('Acme one')).toBeTruthy();
  fireEvent.click(screen.getByText('More'));
  expect(await screen.findByText('Acme two')).toBeTruthy();
});

test('a refused edit shows the field error from the 422 body', async () => {
  server.use(
    getCard(),
    updateCardHandler({ status: 422, body: { fields: { title: 'Must not be empty' } } }),
  );
  app(<CardEditor cardId="a1" />);
  const input = await screen.findByDisplayValue('Acme one');
  fireEvent.change(input, { target: { value: '' } });
  fireEvent.click(screen.getByText('Save'));
  expect((await screen.findByRole('alert')).textContent).toBe('Must not be empty');
  // The optimistic title was rolled back.
  expect(screen.getByRole('heading').textContent).toBe('Acme one');
});
