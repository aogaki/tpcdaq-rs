// =============================================================================
// graw2root.C - step 1: PURE conversion of a mini-eTPC GRAW file to ROOT.
//
// No analysis at all: no pedestal subtraction, no threshold, no geometry -
// the full waveforms are stored losslessly. Everything else happens in
// analyzeUVW.C, so re-tuning cuts never requires re-reading the GRAW file.
//
// Standalone: NO GET software needed. CoBo full-readout frames (revision 5,
// single AsAd) are decoded directly:
//   * every frame: 256-byte big-endian header + 32-bit items
//   * item bits:  [31:30]=AGET  [29:23]=channel  [22:14]=time cell  [11:0]=ADC
//   * 12-byte "topology" frames (written at every DAQ start) are skipped
//
// Output: ONE <run>_raw.root with TTree "raw" (one entry per event):
//   eventId              event number from the frame header
//   eventTime            48-bit CoBo timestamp (clock ticks)
//   adc[4][68][512]      raw ADC of every AGET / channel / time cell
//
// Input can be:
//   * one chunk:      graw2root.C("CoBo_xxx_0000.graw")
//                     -> follow-up chunks _0001, _0002, ... are found and
//                        merged automatically (stop at the first gap)
//   * a list file:    graw2root.C("rawFileList.dat")
//                     -> one GRAW path per line, '#' comments allowed
//
// C++: plain STL streams/strings only (works interpreted and with ACLiC;
// std::filesystem is deliberately avoided for old-cluster portability).
//
// Usage:  root -l -b -q 'graw2root.C("CoBo_xxx_0000.graw", -1)'
// =============================================================================
#include <TFile.h>
#include <TTree.h>

#include <cstring>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <sstream>
#include <string>
#include <vector>

namespace
{
bool fileExists(const std::string &path)
{
  return std::ifstream(path).good();
}

// trim whitespace on both sides
std::string trim(const std::string &s)
{
  auto b = s.find_first_not_of(" \t\r\n");
  auto e = s.find_last_not_of(" \t\r\n");
  return b == std::string::npos ? "" : s.substr(b, e - b + 1);
}

bool endsWith(const std::string &s, const std::string &suffix)
{
  return s.size() >= suffix.size() &&
         s.compare(s.size() - suffix.size(), suffix.size(), suffix) == 0;
}
}  // namespace

void graw2root(const std::string &input, Long64_t maxEvents = -1)
{
  // guard against the most likely mix-up: feeding a ROOT file back in
  if (endsWith(input, ".root")) {
    std::cout << "'" << input << "' is a ROOT file.\n"
              << "graw2root.C takes a .graw file (or a text list of them).\n"
              << "For *_raw.root use analyzeUVW.C; for *_uvw.root use "
                 "makeTracks.C." << std::endl;
    return;
  }

  // --- build the list of GRAW files to merge ---------------------------------
  std::vector<std::string> files;
  if (endsWith(input, ".graw")) {
    files.push_back(input);
    // auto-discover the following chunks: count the NNNN suffix upwards
    const std::string stem = input.substr(0, input.size() - 9);  // keep '_'
    int chunk = std::stoi(input.substr(input.size() - 9, 4));
    for (;;) {
      std::ostringstream next;
      next << stem << std::setw(4) << std::setfill('0') << ++chunk << ".graw";
      if (!fileExists(next.str())) break;
      files.push_back(next.str());
    }
  } else {  // a text file with one GRAW path per line
    std::ifstream list(input);
    if (!list) {
      std::cout << "cannot open " << input << std::endl;
      return;
    }
    for (std::string line; std::getline(list, line);) {
      const std::string path = trim(line);
      if (!path.empty() && path[0] != '#') files.push_back(path);
    }
  }
  if (files.empty()) {
    std::cout << "no input files" << std::endl;
    return;
  }
  std::cout << "merging " << files.size() << " GRAW file(s)" << std::endl;

  // --- output tree (named after the first file, chunk suffix dropped) --------
  std::string outName = files.front();
  for (auto &ch : outName)
    if (ch == ':') ch = '-';  // ROOT dislikes ':' in paths
  if (files.size() > 1)
    outName.replace(outName.size() - 10, 10, "_raw.root");  // xxx_0000.graw
  else
    outName.replace(outName.size() - 5, 5, "_raw.root");    // xxx.graw
  TFile out(outName.c_str(), "RECREATE");
  TTree tree("raw", "mini-eTPC raw waveforms");
  UInt_t eventId;
  ULong64_t eventTime;
  static Short_t adc[4][68][512];
  tree.Branch("eventId", &eventId);
  tree.Branch("eventTime", &eventTime);
  tree.Branch("adc", adc, "adc[4][68][512]/S");

  // --- loop over files and frames ---------------------------------------------
  unsigned char hdr[256];
  Long64_t nEvents = 0;

  for (const std::string &grawFile : files) {
    if (nEvents == maxEvents) break;
    std::ifstream f(grawFile, std::ios::binary);
    if (!f) {
      std::cout << "cannot open " << grawFile << std::endl;
      return;
    }
    std::cout << "reading " << grawFile << std::endl;
    const Long64_t nBefore = nEvents;

    while (f.read(reinterpret_cast<char *>(hdr), 12)) {
      // frame size [bytes] = 3-byte count * block size (2^(metaType & 0xF))
      const long blob = 1L << (hdr[0] & 0x0F);
      const long nbytes = ((hdr[1] << 16) | (hdr[2] << 8) | hdr[3]) * blob;
      const int ftype = (hdr[5] << 8) | hdr[6];
      if (ftype != 1) {  // skip topology and other non-data frames
        f.seekg(nbytes - 12, std::ios::cur);
        continue;
      }
      if (!f.read(reinterpret_cast<char *>(hdr) + 12, 256 - 12)) break;
      eventId = (hdr[22] << 24) | (hdr[23] << 16) | (hdr[24] << 8) | hdr[25];
      eventTime = 0;
      for (int b = 16; b < 22; ++b) eventTime = (eventTime << 8) | hdr[b];

      // decode all items of this frame into the ADC array
      std::memset(adc, 0, sizeof(adc));
      const long nItems = (nbytes - 256) / 4;
      std::vector<unsigned char> buf(nbytes - 256);
      if (!f.read(reinterpret_cast<char *>(buf.data()), buf.size())) break;
      for (long i = 0; i < nItems; ++i) {
        const unsigned int w = (buf[4 * i] << 24) | (buf[4 * i + 1] << 16) |
                               (buf[4 * i + 2] << 8) | buf[4 * i + 3];
        adc[(w >> 30) & 0x3][(w >> 23) & 0x7F][(w >> 14) & 0x1FF] = w & 0xFFF;
      }
      tree.Fill();
      if (++nEvents == maxEvents) break;
    }
    if (nEvents == nBefore)
      std::cout << "WARNING: no events in " << grawFile
                << " (empty file, or a DAQ false start?)" << std::endl;
  }
  tree.Write();
  out.Close();
  std::cout << "wrote " << nEvents << " events to " << outName << std::endl;
}
