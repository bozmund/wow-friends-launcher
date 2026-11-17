// SPDX-License-Identifier: GPL-2.0-only
// Native Wine-compatible adaptation of the key-table algorithm in
// Karrz1/Karrz-playerbots-wow-forever v0.1.37.3, repack/Play-WoWForever.ps1.
// Upstream update ZIP SHA256:
// ad2bc3d3396621ce8329bbb9c152bf938e97d8b297a2d35030b59cbca5b8f40e
// Only the explicitly launched child game is inspected. No game files are patched.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <wincrypt.h>
#include <winsock2.h>
#include <ws2tcpip.h>
#include <tlhelp32.h>
#include <algorithm>
#include <array>
#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

constexpr size_t EntrySize = 40, Entries = 12, BlockSize = EntrySize * Entries;
constexpr unsigned char Flags[Entries] = {1,1,1,1,1,1,1,1,0,1,1,1};
constexpr unsigned char PublicKey[32] = {
  0x02,0x59,0x6f,0x0d,0x0c,0x06,0x1a,0x8b,0x30,0x74,0x59,0x88,0xfd,0x72,0xc5,0x9e,
  0x29,0xec,0x36,0x7f,0xb0,0xf3,0x41,0xf2,0x8e,0x0f,0x08,0xd0,0x37,0xba,0xfc,0x69};

bool looks(const unsigned char* b, size_t entries=Entries) {
  for (size_t e=0; e<entries; ++e) {
    const auto* p=b+e*EntrySize;
    uint32_t id=uint32_t(p[0])|(uint32_t(p[1])<<8)|(uint32_t(p[2])<<16)|(uint32_t(p[3])<<24);
    bool flagValid=e==8 ? p[36]<=1 : p[36]==Flags[e];
    if(id!=e+1 || !flagValid || p[38] || p[39] || (p[37] && p[37]!=0x7f)) return false;
    bool nonzero=false;
    for(size_t k=4;k<36;++k) nonzero|=p[k]!=0;
    if(!nonzero && p[36]) return false;
  }
  return true;
}

int state(HANDLE process, uintptr_t table) {
  std::array<unsigned char,BlockSize> b{};SIZE_T got=0;
  if(!ReadProcessMemory(process,reinterpret_cast<void*>(table),b.data(),b.size(),&got) || got!=b.size() || !looks(b.data())) return -1;
  return std::equal(std::begin(PublicKey),std::end(PublicKey),b.data()+7*EntrySize+4)?1:0;
}

bool apply(HANDLE process, uintptr_t table) {
  SIZE_T written=0;
  return WriteProcessMemory(process,reinterpret_cast<void*>(table+7*EntrySize+4),PublicKey,sizeof(PublicKey),&written)
      && written==sizeof(PublicKey) && state(process,table)==1;
}

std::vector<uintptr_t> find(HANDLE process, bool inspect=false) {
  std::vector<uintptr_t> found;
  size_t regions=0,readable=0,failures=0,scanned=0;DWORD readError=0;
  uintptr_t address=0;MEMORY_BASIC_INFORMATION info{};
  while(address<0x7fffffff0000ULL && VirtualQueryEx(process,reinterpret_cast<void*>(address),&info,sizeof(info))) {
    auto base=reinterpret_cast<uintptr_t>(info.BaseAddress);auto size=info.RegionSize;
    DWORD protection=info.Protect&0xff;
    bool eligible=inspect ? (protection==PAGE_READONLY || protection==PAGE_READWRITE || protection==PAGE_WRITECOPY) && !(info.Protect&PAGE_GUARD)
                          : (info.Type==MEM_PRIVATE || info.Type==MEM_MAPPED) && info.Protect==PAGE_READWRITE;
    if(info.State==MEM_COMMIT && eligible) {
      ++regions;
      size_t pos=0;
      while(pos<size) {
        size_t take=std::min<size_t>(8*1024*1024,size-pos),got=0;
        std::vector<unsigned char> bytes(take);
        if(!ReadProcessMemory(process,reinterpret_cast<void*>(base+pos),bytes.data(),take,&got)) {++failures;readError=GetLastError();break;}
        ++readable;scanned+=got;
        for(size_t i=0;i+BlockSize<=got;i+=4)
          if(bytes[i]==1 && bytes[i+1]==0 && bytes[i+40]==2 && looks(bytes.data()+i,inspect?8:Entries)) {
            found.push_back(base+pos+i);
            if(inspect) {
              printf("Eight-region key-table candidate: address=%llx type=%lx protect=%lx full12=%d currentServerKey=%d\n",static_cast<unsigned long long>(base+pos+i),info.Type,info.Protect,looks(bytes.data()+i),state(process,base+pos+i));
              for(size_t row=0;row<Entries;++row) {
                auto* p=bytes.data()+i+row*EntrySize;uint32_t id=uint32_t(p[0])|(uint32_t(p[1])<<8)|(uint32_t(p[2])<<16)|(uint32_t(p[3])<<24);
                bool nonzero=false;for(size_t k=4;k<36;++k) nonzero|=p[k]!=0;
                printf("  row=%zu idMatches=%d flagMatches=%d paddingMatches=%d keyPresent=%d\n",row+1,id==row+1,p[36]==Flags[row],p[38]==0 && p[39]==0 && (p[37]==0 || p[37]==0x7f),nonzero);
              }
            }
          }
        if(pos+take>=size || take<=BlockSize) break;
        pos+=take-BlockSize;
      }
    }
    if(!size || base+size<=address) break;
    address=base+size;
  }
  static unsigned samples=0;
  if((samples++%8)==0) printf("Key scan: private regions=%zu reads=%zu failures=%zu scannedMiB=%zu matches=%zu lastError=%lu\n",regions,readable,failures,scanned/(1024*1024),found.size(),readError);
  return found;
}

int trust_ca(const wchar_t* path) {
  HANDLE file=CreateFileW(path,GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL,nullptr);
  if(file==INVALID_HANDLE_VALUE) return 20;
  DWORD size=GetFileSize(file,nullptr),got=0;
  if(size==INVALID_FILE_SIZE || size>1024*1024) {CloseHandle(file);return 21;}
  std::vector<char> pem(size);
  bool read=ReadFile(file,pem.data(),size,&got,nullptr);CloseHandle(file);
  if(!read || got!=size) return 22;
  DWORD count=0;
  if(!CryptStringToBinaryA(pem.data(),size,CRYPT_STRING_BASE64HEADER,nullptr,&count,nullptr,nullptr)) return 23;
  std::vector<unsigned char> der(count);
  if(!CryptStringToBinaryA(pem.data(),size,CRYPT_STRING_BASE64HEADER,der.data(),&count,nullptr,nullptr)) return 24;
  for(DWORD location : {DWORD(CERT_SYSTEM_STORE_LOCAL_MACHINE),DWORD(CERT_SYSTEM_STORE_CURRENT_USER)}) {
    HCERTSTORE store=CertOpenStore(CERT_STORE_PROV_SYSTEM_W,0,0,location,L"ROOT");
    if(!store) return 25;
    bool added=CertAddEncodedCertificateToStore(store,X509_ASN_ENCODING|PKCS_7_ASN_ENCODING,der.data(),count,CERT_STORE_ADD_REPLACE_EXISTING,nullptr);
    CertCloseStore(store,0);
    if(!added) return 26;
  }
  puts("Private game CA installed in this Wine prefix's machine and current-user stores; Linux trust is unchanged.");return 0;
}

// Adapted from current upstream CertKeys: only exact known public keys in
// committed private writable data; executable pages and files are untouched.
constexpr unsigned char OriginalKeys[12][32] = {
  {0x9b,0x06,0x71,0xc8,0x15,0xdf,0xf5,0x13,0xbf,0xd4,0xa2,0xb2,0x6a,0xe1,0xf8,0x4e,0xc9,0x10,0x68,0x41,0xb2,0xfb,0x62,0x0d,0xb6,0x5f,0x6a,0xde,0x7c,0x21,0xad,0x06},
  {0x11,0x24,0x51,0xb9,0x84,0x3c,0x27,0x99,0xe4,0x75,0x0a,0xa3,0xd9,0xb9,0xaf,0xab,0xf5,0x36,0xa6,0x45,0xb8,0x63,0xc4,0xad,0xb2,0x07,0x8b,0x93,0x2b,0x35,0x4e,0x04},
  {0xa6,0xb8,0x58,0x48,0x57,0x48,0xbf,0x38,0xbe,0x19,0x35,0x17,0xab,0x90,0xf8,0xde,0x16,0x9e,0xff,0x09,0x89,0xea,0x93,0x60,0xdb,0x34,0x6a,0x37,0x8b,0x0f,0xfe,0x15},
  {0x50,0xff,0xde,0xb8,0x07,0xf8,0xff,0x73,0xa2,0x99,0xa1,0x60,0x00,0xaa,0x65,0x75,0xc5,0x94,0x5b,0xe8,0x75,0xad,0xfc,0x87,0x95,0x37,0x4e,0xd3,0x41,0x52,0x49,0xac},
  {0x9f,0x84,0x2d,0x07,0x87,0x55,0x64,0x7c,0x00,0x8e,0x5f,0xe3,0xe1,0x2b,0x83,0x9a,0x98,0x1d,0xe0,0x2a,0x1f,0x52,0x0c,0xb2,0x54,0x5c,0xc0,0x4c,0xd2,0x32,0x3e,0x0e},
  {0xb8,0xcc,0x75,0x86,0x4d,0x8e,0xf4,0x61,0xd8,0xdd,0x6a,0xa7,0x42,0x5e,0x2c,0x63,0xc2,0x4d,0x39,0x82,0x06,0x6b,0x8d,0x77,0x3a,0x15,0x49,0xbf,0xe2,0x4e,0x05,0xee},
  {0x76,0xbb,0x7b,0xbd,0xd9,0xf3,0x4e,0x12,0x4a,0x45,0x73,0xc3,0xaa,0x22,0x7e,0x3c,0x87,0xcc,0x60,0x35,0x06,0x69,0x7d,0x05,0x4f,0x6f,0xde,0xc3,0x42,0xe5,0x6e,0xbf},
  {0x1f,0xd6,0xdd,0x8f,0xa0,0xec,0x30,0xd3,0x9e,0x3f,0x72,0xe7,0x55,0xb8,0xa0,0x45,0xbd,0xe0,0xf7,0x04,0x49,0xdc,0x71,0x00,0x8b,0x76,0x7c,0x2e,0xaa,0x89,0xfb,0x9f},
  {0x15,0xd6,0x18,0xbd,0x7d,0xb5,0x77,0xbd,0x9a,0x8d,0x45,0x76,0x9c,0x59,0xe4,0xfc,0x63,0x16,0x33,0xbf,0x44,0x73,0x98,0xa4,0xb4,0x89,0xb4,0xc2,0x6f,0xbc,0x03,0xad},
  {0xb3,0x4e,0xc5,0x92,0xd2,0xac,0x49,0x93,0xf5,0xff,0xc8,0x5b,0x15,0xc3,0xda,0x90,0x78,0x69,0x40,0x51,0xcb,0x22,0x43,0x45,0x59,0x2a,0xf7,0x13,0x6d,0x79,0x6c,0x99},
  {0x9e,0x91,0x58,0x6d,0xd4,0xb1,0x15,0xab,0x05,0x68,0xb2,0x19,0x59,0xe1,0x81,0xf6,0x54,0x59,0x10,0xbc,0x5e,0x2f,0x30,0xb8,0x97,0x5c,0xa0,0x9d,0x7b,0xc3,0xef,0xce},
  {0x76,0x9c,0x82,0x4a,0x1d,0xad,0xd1,0x9a,0xee,0xfa,0x42,0x04,0x14,0xd1,0x73,0x2d,0xaf,0x44,0x53,0x7f,0x98,0xaa,0x22,0x9f,0xc5,0x47,0x7b,0xca,0x55,0xf9,0xd5,0x9a}
};
const unsigned char* original_key(const unsigned char* bytes) {
  static const auto prefixes=[] {std::array<unsigned char,256> p{};for(size_t i=0;i<12;++i) p[OriginalKeys[i][0]]=static_cast<unsigned char>(i+1);return p;}();
  auto index=prefixes[bytes[0]];
  if(index && std::equal(std::begin(OriginalKeys[index-1]),std::end(OriginalKeys[index-1]),bytes)) return OriginalKeys[index-1];
  return nullptr;
}
std::vector<uintptr_t> cachedKeys;

int key_copies(HANDLE process, bool repair) {
  size_t originals=0, ours=0, written=0;uintptr_t address=0;MEMORY_BASIC_INFORMATION info{};
  while(address<0x7fffffff0000ULL && VirtualQueryEx(process,reinterpret_cast<void*>(address),&info,sizeof(info))) {
    auto base=reinterpret_cast<uintptr_t>(info.BaseAddress);auto size=info.RegionSize;
    if(info.State==MEM_COMMIT && info.Type==MEM_PRIVATE && info.Protect==PAGE_READWRITE) {
      for(size_t pos=0;pos<size;) {
        size_t take=std::min<size_t>(4*1024*1024,size-pos),got=0;std::vector<unsigned char> bytes(take);
        if(!ReadProcessMemory(process,reinterpret_cast<void*>(base+pos),bytes.data(),take,&got)) break;
        for(size_t i=0;i+32<=got;++i) {
          if(bytes[i]==PublicKey[0] && std::equal(std::begin(PublicKey),std::end(PublicKey),bytes.data()+i)) {++ours;if(repair) cachedKeys.push_back(base+pos+i);}
          const auto* original=original_key(bytes.data()+i);if(!original) continue;
          ++originals;
          if(repair) {
            std::array<unsigned char,32> check{};SIZE_T count=0;
            auto target=reinterpret_cast<void*>(base+pos+i);
            if(ReadProcessMemory(process,target,check.data(),32,&count) && count==32 && std::equal(check.begin(),check.end(),original)
               && WriteProcessMemory(process,target,PublicKey,32,&count) && count==32
               && ReadProcessMemory(process,target,check.data(),32,&count) && count==32 && std::equal(check.begin(),check.end(),std::begin(PublicKey))) {++written;cachedKeys.push_back(base+pos+i);}
          }
          i+=31;
        }
        if(pos+take>=size || take<=32) break;
        pos+=take-31;
      }
    }
    if(!size || base+size<=address) break;
    address=base+size;
  }
  printf("Private known public-key copies: original=%zu current=%zu verifiedWrites=%zu repair=%d\n",originals,ours,written,repair);
  return repair && written!=originals ? 53 : 0;
}

void guard_copies(HANDLE process) {
  static ULONGLONG lastScan=0;auto now=GetTickCount64();
  if(now-lastScan>=5000) {cachedKeys.clear();key_copies(process,true);lastScan=GetTickCount64();return;}
  for(auto it=cachedKeys.begin();it!=cachedKeys.end();) {
    std::array<unsigned char,32> check{};SIZE_T count=0;auto target=reinterpret_cast<void*>(*it);
    if(!ReadProcessMemory(process,target,check.data(),32,&count) || count!=32) {it=cachedKeys.erase(it);continue;}
    if(original_key(check.data())) {
      if(WriteProcessMemory(process,target,PublicKey,32,&count) && count==32) puts("Cached public key refreshed.");
    } else if(!std::equal(check.begin(),check.end(),std::begin(PublicKey))) {it=cachedKeys.erase(it);continue;}
    ++it;
  }
}

int self_test() {
  std::array<unsigned char,BlockSize> table{};
  for(size_t e=0;e<Entries;++e) {table[e*EntrySize]=static_cast<unsigned char>(e+1);table[e*EntrySize+4]=1;table[e*EntrySize+36]=Flags[e];}
  if(!looks(table.data())) return 30;
  table[8*EntrySize+36]=1;if(!looks(table.data())) return 33;
  table[8*EntrySize+4]=0;if(looks(table.data())) return 34;
  table[8*EntrySize+36]=0;if(!looks(table.data())) return 35;
  table[8*EntrySize+4]=1;
  table[40]=3;if(looks(table.data())) return 31;table[40]=2;
  auto p=reinterpret_cast<uintptr_t>(table.data());
  if(state(GetCurrentProcess(),p)!=0 || !apply(GetCurrentProcess(),p) || state(GetCurrentProcess(),p)!=1) return 32;
  puts("Key-table validation and read/write self-test passed.");return 0;
}

int wmain(int argc,wchar_t** argv) {
  setvbuf(stdout,nullptr,_IONBF,0);
  if(argc==2 && (std::wstring(argv[1])==L"--inspect-game" || std::wstring(argv[1])==L"--watch-game-key" || std::wstring(argv[1])==L"--inspect-key-copies" || std::wstring(argv[1])==L"--repair-key-copies")) {
    bool watch=std::wstring(argv[1])==L"--watch-game-key";
    bool repair=std::wstring(argv[1])==L"--repair-key-copies";
    HANDLE snapshot=CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS,0);
    if(snapshot==INVALID_HANDLE_VALUE) return 50;
    PROCESSENTRY32W entry{};entry.dwSize=sizeof(entry);DWORD pid=0;unsigned count=0;
    if(Process32FirstW(snapshot,&entry)) do {
      if(_wcsicmp(entry.szExeFile,L"Wow.exe")==0) {pid=entry.th32ProcessID;++count;}
    } while(Process32NextW(snapshot,&entry));
    CloseHandle(snapshot);
    if(count!=1) {printf("Expected one WoW client in this Wine prefix, found %u.\n",count);return 51;}
    HANDLE process=OpenProcess(PROCESS_QUERY_INFORMATION|PROCESS_VM_READ|((watch||repair)?(PROCESS_VM_WRITE|PROCESS_VM_OPERATION|SYNCHRONIZE):0),FALSE,pid);
    if(!process) {printf("Could not inspect client, error %lu.\n",GetLastError());return 52;}
    if(repair || std::wstring(argv[1])==L"--inspect-key-copies") {int result=key_copies(process,repair);CloseHandle(process);return result;}
    if(watch) {
      printf("Watching public login key for the single existing WoW client %lu.\n",pid);
      uintptr_t table=0;
      while(WaitForSingleObject(process,0)==WAIT_TIMEOUT) {
        if(table) {
          int current=state(process,table);
          if(current==1) {Sleep(2000);continue;}
          if(current==0 && apply(process,table)) {puts("Public server login key reapplied and verified.");continue;}
          table=0;
        }
        auto candidates=find(process);
        if(candidates.size()==1 && apply(process,candidates.front())) {table=candidates.front();puts("Public server login key applied and verified.");}
        Sleep(1000);
      }
      CloseHandle(process);return 0;
    }
    printf("Inspecting only WoW client Windows process %lu (read only).\n",pid);
    auto tables=find(process,true);CloseHandle(process);
    printf("Read-only inspection found %zu validated public-key tables.\n",tables.size());return 0;
  }
  if(argc==2 && std::wstring(argv[1])==L"--check-dns") {
    WSADATA data{};if(WSAStartup(MAKEWORD(2,2),&data)) return 40;
    addrinfo hint{};hint.ai_family=AF_INET;hint.ai_socktype=SOCK_STREAM;addrinfo* result=nullptr;
    int code=getaddrinfo("trinity.actual.battle.net","1119",&hint,&result);
    if(code || !result) {WSACleanup();puts("Private game hostname did not resolve.");return 41;}
    bool local=false;
    for(auto* p=result;p;p=p->ai_next) {
      auto* address=reinterpret_cast<sockaddr_in*>(p->ai_addr);
      local|=address->sin_addr.s_addr==htonl(INADDR_LOOPBACK);
    }
    freeaddrinfo(result);WSACleanup();
    if(!local) return 42;
    puts("Private game hostname resolves to loopback inside Wine.");return 0;
  }
  if(argc==2 && std::wstring(argv[1])==L"--self-test") return self_test();
  if(argc==3 && std::wstring(argv[1])==L"--trust-ca") return trust_ca(argv[2]);
  if(argc!=3 || std::wstring(argv[1])!=L"--launch") {puts("Usage: --self-test | --trust-ca FILE | --launch FULL_PATH_TO_Wow.exe");return 2;}
  std::wstring exe=argv[2];auto slash=exe.find_last_of(L"/\\");
  if(slash==std::wstring::npos || _wcsicmp(exe.substr(slash+1).c_str(),L"Wow.exe")!=0 || exe.find(L'"')!=std::wstring::npos) return 3;
  std::wstring command=L"\""+exe+L"\" -config ForeverLauncher.wtf -d3d11";
  STARTUPINFOW startup{};startup.cb=sizeof(startup);PROCESS_INFORMATION child{};
  if(!CreateProcessW(exe.c_str(),command.data(),nullptr,nullptr,FALSE,0,nullptr,exe.substr(0,slash).c_str(),&startup,&child)) {
    printf("Game launch failed: %lu\n",GetLastError());return 4;
  }
  CloseHandle(child.hThread);puts("Private game client started; updating private working keys only. Image and mapped regions stay unchanged.");
  while(WaitForSingleObject(child.hProcess,0)==WAIT_TIMEOUT) {
    guard_copies(child.hProcess);
    Sleep(250);
  }
  DWORD code=0;GetExitCodeProcess(child.hProcess,&code);CloseHandle(child.hProcess);
  printf("Game closed with exit code %lu.\n",code);return code?5:0;
}
