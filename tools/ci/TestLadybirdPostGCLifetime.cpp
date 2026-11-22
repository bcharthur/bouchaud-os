#include <LibGC/Cell.h>
#include <LibGC/CellAllocator.h>
#include <LibGC/Heap.h>
#include <LibGC/Root.h>
#include <LibGC/WeakInlines.h>
#include <LibTest/TestCase.h>

namespace {

class Target final : public GC::Cell {
    GC_CELL(Target, GC::Cell);
    GC_DECLARE_ALLOCATOR(Target);

public:
    unsigned calls { 0 };
    u8 padding[16] {};
};

GC_DEFINE_ALLOCATOR(Target);

struct Observation {
    unsigned finalized { 0 };
    unsigned callbacks { 0 };
    unsigned suppressed { 0 };
};

class Resource final : public GC::Cell {
    GC_CELL(Resource, GC::Cell);
    GC_DECLARE_ALLOCATOR(Resource);

public:
    Resource(Target& target, Observation& observation)
        : m_target(target)
        , m_observation(observation)
    {
    }

    void finalize() override
    {
        Base::finalize();
        ++m_observation.finalized;
        heap().enqueue_post_gc_task([target = m_target, &observation = m_observation] {
            ++observation.callbacks;
            if (target)
                ++target->calls;
            else
                ++observation.suppressed;
        });
    }

private:
    GC::Weak<Target> m_target;
    Observation& m_observation;
};

GC_DEFINE_ALLOCATOR(Resource);

}

TEST_CASE(post_gc_task_observes_target_lifetime_across_collections)
{
    Observation observation;
    GC::Heap heap([](auto&) { }, GC::Heap::BecomeProcessDefault::No);
    heap.set_incremental_sweep_enabled(false);
    auto target = GC::make_root(heap.allocate<Target>());
    for (unsigned pass = 1; pass <= 3; ++pass) {
        auto resource = heap.allocate<Resource>(*target, observation);
        heap.uproot_cell(resource.ptr());
        heap.collect_garbage();
        EXPECT_EQ(observation.finalized, pass);
        EXPECT_EQ(observation.callbacks, pass);
        EXPECT_EQ(target->calls, pass);
        EXPECT_EQ(observation.suppressed, 0u);
    }
    auto resource = heap.allocate<Resource>(*target, observation);
    heap.uproot_cell(resource.ptr());
    heap.uproot_cell(target.ptr());
    target = GC::Root<Target> {};
    heap.collect_garbage();
    EXPECT_EQ(observation.finalized, 4u);
    EXPECT_EQ(observation.callbacks, 4u);
    EXPECT_EQ(observation.suppressed, 1u);
}

TEST_CASE(post_gc_task_does_not_keep_target_alive_in_heap_teardown)
{
    Observation observation;
    GC::Heap heap([](auto&) { }, GC::Heap::BecomeProcessDefault::No);
    heap.set_incremental_sweep_enabled(false);
    auto target = heap.allocate<Target>();
    (void)heap.allocate<Resource>(*target, observation);
    heap.collect_garbage(GC::Heap::CollectionType::CollectEverything);
    EXPECT_EQ(observation.finalized, 1u);
    EXPECT_EQ(observation.callbacks, 1u);
    EXPECT_EQ(observation.suppressed, 1u);
}
