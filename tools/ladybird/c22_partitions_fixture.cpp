// C22 -- banc hote (BOUCHAUD_C22_PARTITIONS_COLLECTEES_V1). Le motif de
// WebContent : des documents allouent noeuds de layout et paintables dans
// leurs tas partitionnes, sont detruits, puis le balayage GC appelle
// ak_kmalloc_collect(). Deux phases identiques ; RSS releve apres le crochet.
// Construit et juge par tools/ladybird/test-c22-partitions.sh.
#include <AK/kmalloc.h>
#include <cstdio>
#include <cstring>
#include <vector>
#include <unistd.h>
static long rss_kio() { long p=0,r=0; FILE* f=fopen("/proc/self/statm","r"); if (!f) return -1; int n=fscanf(f,"%ld %ld",&p,&r); fclose(f); return n == 2 ? r*4 : -1; }
int main()
{
    std::vector<void*> vivants;
    for (int i = 0; i < 2000; ++i) vivants.push_back(ak_kmalloc(HeapPartition::General, 100));
    long base = rss_kio();
    for (int phase = 1; phase <= 2; ++phase) {
        for (int onglet = 0; onglet < 10; ++onglet) {
            std::vector<void*> doc;
            for (int i = 0; i < 30000; ++i) {
                size_t t = 32 + (i * 37 % 900);
                doc.push_back(ak_kmalloc(i % 3 == 0 ? HeapPartition::Painting : HeapPartition::Layout, t));
                memset(doc.back(), 1, t);
                if (i % 50 == 0) vivants.push_back(ak_kmalloc(HeapPartition::General, 64));
            }
            for (auto* p : doc) ak_kfree(p);
        }
        long avant = rss_kio();
        ak_kmalloc_collect();
        printf("C22_HOTE phase=%d avant_collecte_kio=%ld retenu_kio=%ld base_kio=%ld\n", phase, avant - base, rss_kio() - base, base);
    }
}
