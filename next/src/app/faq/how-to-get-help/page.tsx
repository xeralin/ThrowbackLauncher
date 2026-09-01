import type { Metadata } from "next";
import { FAQ_PAGES } from "@/config/faq";
import { FaqHero } from "@/components/FaqHero";
import { SectionTitle } from "@/components/SectionTitle";
import { Prose } from "@/components/Prose";
import { Note } from "@/components/Note";
import { ExternalLink } from "@/components/ExternalLink";
import { site } from "@/config/site";

export const metadata: Metadata = FAQ_PAGES["how-to-get-help"];

export default function HowToGetHelp() {
  return (
    <>
      <FaqHero page="how-to-get-help" />

      <Note className="mb-6">
        If you run into errors, attach the <code>bin/log.txt</code> file to your
        report.
      </Note>

      <SectionTitle>Reporting an Issue to Staff</SectionTitle>
      <Prose>
        <p>
          If your issue is not covered in the FAQ, join the{" "}
          <ExternalLink href={site.discordUrl}>Discord server</ExternalLink> and
          create a post in{" "}
          <ExternalLink href={site.helpChannelUrl}>
            <code>#help</code>
          </ExternalLink>
          .
        </p>
        <h3>Title</h3>
        <p>Write a short, clear title that summarizes the issue.</p>

        <h3>Description</h3>
        <p>Describe the problem in detail and include the following points.</p>
        <ul>
          <li>What you were doing when the issue occurred</li>
          <li>Any steps you have already tried</li>
          <li>Any error messages or unusual behavior</li>
        </ul>

        <h3>Screenshots</h3>
        <p>
          Attach screenshots where relevant. Use the built-in screenshot tool on
          your computer. Do <strong>NOT</strong> take photos of your screen with
          a phone.
        </p>

        <h3>Notifying Staff</h3>
        <p>
          After posting, ping the <strong>Helper</strong> role once. Do not tag
          individual staff members directly. If you do, you will most likely be
          ignored or made fun of.
        </p>
      </Prose>
    </>
  );
}
