import type { ReactNode } from 'react';
import { Providers } from './providers';
import { requestSession } from './session';

export default async function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>
        <Providers session={await requestSession()}>{children}</Providers>
      </body>
    </html>
  );
}
