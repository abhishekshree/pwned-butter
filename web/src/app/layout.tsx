import type { Metadata, Viewport } from "next";
import { Public_Sans, Source_Serif_4 } from "next/font/google";
import { ThemeProvider } from "next-themes";

import "./globals.css";

const fontSans = Public_Sans({
  variable: "--font-sans",
  subsets: ["latin"],
  display: "swap",
});

const fontSerif = Source_Serif_4({
  variable: "--font-serif",
  subsets: ["latin"],
  display: "swap",
});

export const metadata: Metadata = {
  title: "The Food Safety Record",
  description:
    "Maharashtra FDA enforcement actions — suspensions, seizures and reopenings, sourced from public reporting.",
};

export const viewport: Viewport = {
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#f4f6f5" },
    { media: "(prefers-color-scheme: dark)", color: "#101715" },
  ],
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html
      lang="en"
      className={`${fontSans.variable} ${fontSerif.variable} h-full antialiased`}
      suppressHydrationWarning
    >
      <body className="flex min-h-full flex-col bg-background font-sans text-foreground selection:bg-primary/20 selection:text-primary">
        <a href="#main" className="skip-link">
          Skip to content
        </a>
        <ThemeProvider attribute="class" defaultTheme="system" enableSystem>
          {children}
        </ThemeProvider>
      </body>
    </html>
  );
}
