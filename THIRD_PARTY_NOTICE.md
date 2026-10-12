# Third-party and commercial-content notice

CyberMage: Darklight Awakening is a commercial game originally published by
Origin Systems in 1995. The game, original executable, artwork, music, speech,
movies and trademarks remain the property of their respective rights holders.
CyberMage: Darklight Re-Awakening, an independent, unofficial compatibility
engine, is not affiliated with or endorsed by Origin Systems or Electronic
Arts. Users must supply their own lawfully obtained CyberMage v1.01F game files. No commercial game files are
included in the portable release, and its importer does not download any.
The Re-Awakening title-screen wordmark is re-set at runtime from the user's own
title art. The executable embeds `assets/cybermage.ico`, a cropped Darklight
emblem from the original title artwork, solely as its Windows application icon.
This icon is an explicit commercial-art exception, retains its original rights,
and is not MIT-licensed artwork or evidence of redistribution permission.
The MIT license covers only the project's original code/documentation and
image-generated control-guide artwork, not commercial game content.

The source repository's landing page includes `docs/img/title.png`, derived
from the original title artwork with a changed subtitle, and nine curated
feature/gameplay screenshots in `docs/img/screenshots/`. The cover retains its
original copyright notice. These images contain commercial artwork and are
explicit retained exceptions, not MIT-licensed artwork or evidence of
permission from the original rights holders. No extracted game fonts or
standalone commercial decorations are included on the landing page. Removing
original pixels from generated cockpit extensions does not itself establish
rights to redistribute those extensions.

The release packager collects copyright/license/notice files from every
resolved dependency for the Windows target into `licenses/` and writes an
inventory in `DEPENDENCIES.txt`. Direct external dependencies:

- minifb 0.29.0: MIT OR Apache-2.0.
- cpal 0.18.2: Apache-2.0.
- libc (Unix builds only): MIT OR Apache-2.0.

The Windows package uses system Windows APIs, XInput and the system General
MIDI device. It does not bundle Windows system DLLs or a MIDI sound bank.
The static C runtime option avoids requiring a separate VC++ Redistributable.

Do not redistribute an installation after importing commercial assets or
creating saves/configuration. Redistribute only the clean ZIP, subject to the icon artwork's original rights.
