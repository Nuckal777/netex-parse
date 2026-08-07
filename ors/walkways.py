from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Any, Callable, List, Mapping, Tuple, TypedDict
import openrouteservice
from openrouteservice.distance_matrix import distance_matrix
from tqdm import tqdm
import pandas as pd
import json
import sys
from LatLon23 import LatLon


# Quadtree implementation kindly taken from: https://github.com/kisv701/quadtree


# Helper functions
def is_point_in_rect(point, bottomLeft, topRight):
    return (
        point.x > bottomLeft.x
        and point.x < topRight.x
        and point.y > bottomLeft.y
        and point.y < topRight.y
    )


def is_point_in_circle(point, circle):
    dst_x = point.x - circle.x
    dst_y = point.y - circle.y

    # Pythagoras to check distance between circle and line to check against.
    return dst_x * dst_x + dst_y * dst_y < circle.value * circle.value


# Class for holding a data point
class Point:
    def __init__(self, x, y, value=None):
        self.x = x
        self.y = y
        self.value = value


# The QuadTree
class QuadTree:
    def __init__(
        self,
        capacity=4,
        bottomLeft=Point(-100, -100, None),
        topRight=Point(100, 100, None),
    ):

        # Quadrants that will be populated as needed
        self.northWest = None
        self.northEast = None
        self.southWest = None
        self.southEast = None

        # Region covered is defined by lower left and upper right corner
        self.bottomLeft = bottomLeft
        self.topRight = topRight

        self.points = []
        self.capacity = capacity

    def add_point(self, point):

        if self.is_inside(point):  # Check if point is within this "quad"
            if (
                self.is_leaf()
            ):  # Check if current quad is leaf, otherwise pass point onto children
                if (
                    len(self.points) < self.capacity
                ):  # If we can take the new point, keep it,
                    self.points.append(point)

                else:  # If this quad can't take point, split this quad.

                    # Step 1, split self into children
                    self.split()

                    # Step 2, Add all the points into the new children
                    for p in self.points:
                        self.northWest.add_point(p)
                        self.northEast.add_point(p)
                        self.southWest.add_point(p)
                        self.southEast.add_point(p)

                    self.northWest.add_point(point)
                    self.northEast.add_point(point)
                    self.southWest.add_point(point)
                    self.southEast.add_point(point)

                    # Step 3, clear points from self.
                    self.points = []

            else:
                # self is not a leaf, pass point onto children
                self.northWest.add_point(point)
                self.northEast.add_point(point)
                self.southWest.add_point(point)
                self.southEast.add_point(point)

    def get_points_in_rect(self, bottomLeft, topRight):

        # If the rect is does not overlap this quad we can't find any points
        if not self.is_overlapping(bottomLeft, topRight):
            return []

        result = []  # All the points in the rectangle

        if self.is_leaf():
            # If we are a leaf node add all the points that fit in the rect
            for p in self.points:
                if is_point_in_rect(p, bottomLeft, topRight):
                    result.append(p)

        else:
            # If we are not a leaf node, add points from all children
            result.extend(self.northWest.get_points_in_rect(bottomLeft, topRight))
            result.extend(self.northEast.get_points_in_rect(bottomLeft, topRight))
            result.extend(self.southWest.get_points_in_rect(bottomLeft, topRight))
            result.extend(self.southEast.get_points_in_rect(bottomLeft, topRight))

        return result

    def get_points_in_circle(self, circle):

        # If the rect is does not overlap this quad we can't find any points
        if not self.is_overlapping_circle(circle):
            return []

        result = []  # All the points in the rectangle

        if self.is_leaf():
            # If we are a leaf node add all the points that fit in the rect
            for p in self.points:
                if is_point_in_circle(p, circle):
                    result.append(p)

        else:
            # If we are not a leaf node, add points from all children
            result.extend(self.northWest.get_points_in_circle(circle))
            result.extend(self.northEast.get_points_in_circle(circle))
            result.extend(self.southWest.get_points_in_circle(circle))
            result.extend(self.southEast.get_points_in_circle(circle))

        return result

    def is_overlapping(self, bottomLeft, topRight):

        # If one is rectangle is to the right of the other they cant overlap
        if self.bottomLeft.x > topRight.x or bottomLeft.x > self.topRight.x:
            return False

        # If one is rectangle is completely above the other they cant overlap
        if self.bottomLeft.y > topRight.y or bottomLeft.y > self.topRight.y:
            return False

        # We only reach here if they are not to the sides of each other and not above each other,
        # so they must overlap
        return True

    def is_overlapping_circle(self, circle):
        # Given a circle, represented by a Point where value is the radius,
        # check if it is overlapping self.

        test_x = circle.x
        test_y = circle.y

        # If the circle is left of the rect, test left edge of rect
        if circle.x < self.bottomLeft.x:
            test_x = self.bottomLeft.x

        # If the circle is right of the rect, test right edge of rect
        elif circle.x > self.topRight.x:
            test_x = self.topRight.x

        # If the circle is below the rect, test bottom edge.
        if circle.y < self.bottomLeft.y:
            test_y = self.bottomLeft.y
        elif circle.y > self.topRight.y:
            test_y = self.topRight.y

        dst_x = test_x - circle.x
        dst_y = test_y - circle.y

        # Pythagoras to check distance between circle and line to check against.
        return dst_x * dst_x + dst_y * dst_y <= circle.value * circle.value

    def split(self):
        x_low = self.bottomLeft.x
        y_low = self.bottomLeft.y
        x_high = self.topRight.x
        y_high = self.topRight.y
        x_mid = self.bottomLeft.x + (self.topRight.x - self.bottomLeft.x) / 2
        y_mid = self.bottomLeft.y + (self.topRight.y - self.bottomLeft.y) / 2
        self.northWest = QuadTree(
            capacity=self.capacity,
            bottomLeft=Point(x_low, y_mid),
            topRight=Point(x_mid, y_high),
        )
        self.northEast = QuadTree(
            capacity=self.capacity,
            bottomLeft=Point(x_mid, y_mid),
            topRight=Point(x_high, y_high),
        )
        self.southWest = QuadTree(
            capacity=self.capacity,
            bottomLeft=Point(x_low, y_low),
            topRight=Point(x_mid, y_mid),
        )
        self.southEast = QuadTree(
            capacity=self.capacity,
            bottomLeft=Point(x_mid, y_low),
            topRight=Point(x_high, y_mid),
        )

    def is_leaf(self):
        return (
            self.northWest is None
            and self.northEast is None
            and self.southWest is None
            and self.southEast is None
        )

    def is_inside(self, point):
        return (
            point.x > self.bottomLeft.x
            and point.y > self.bottomLeft.y
            and point.x < self.topRight.x
            and point.y < self.topRight.y
        )


# End of quadtree implementation


# Runs the given func for each point in the tree, which distance is
# within a rect defined by offset to a center provided. Offset is in km.
# Repeats that for all provided centers.
def for_each_nearby_rect(
    tree: QuadTree,
    centers: List[Point],
    offset: float,
    func: Callable[[Point, Point], None],
):
    for center_point in centers:
        center = LatLon(center_point.y, center_point.x)
        right = center.offset(90, offset)
        left = center.offset(270, offset)
        top = center.offset(0, offset)
        bottom = center.offset(180, offset)
        bottom_left = Point(left.lon.decimal_degree, bottom.lat.decimal_degree)
        top_right = Point(right.lon.decimal_degree, top.lat.decimal_degree)
        nearby: List[Point] = tree.get_points_in_rect(bottom_left, top_right)
        for n in nearby:
            func(center_point, n)


class Edge:
    start: int
    end: int
    duration: float
    distance: float

    def __init__(self, start: int, end: int, duration: float, distance: float):
        self.start = start
        self.end = end
        self.duration = duration
        self.distance = distance


class Request(TypedDict):
    client: openrouteservice.Client
    coords: List[Tuple[float, float]]
    sources_idx: List[int]
    destinations_idx: List[int]
    destinations: List[str]
    source: str


def request_edge(request: Request) -> List[Edge]:
    try:
        matrix = distance_matrix(
            request["client"],
            request["coords"],
            profile="foot-walking",
            metrics=["distance", "duration"],
            sources=request["sources_idx"],
            destinations=request["destinations_idx"],
        )
    except openrouteservice.exceptions.ApiError:
        return []
    edges = []
    for i in range(len(request["destinations"])):
        end = request["destinations"][i]
        duration = matrix["durations"][0][i]
        distance = matrix["distances"][0][i]
        edges.append(Edge(request["source"], end, duration, distance))
    return edges


def chunk(l: List[Any], size: int) -> List[List[Any]]:
    result = []
    current = []
    for idx, item in enumerate(l):
        if idx % size == 0:
            if len(current) != 0:
                result.append(current)
            current = []
        current.append(item)
    if len(current) != 0:
        result.append(current)
    return result


if __name__ == "__main__":
    df = pd.read_csv(sys.argv[1], sep=",", quotechar='"')
    stop_map = {row["id"]: row for idx, row in df.iterrows()}

    min_long = df["lng"].min()
    min_lat = df["lat"].min()
    max_long = df["lng"].max()
    max_lat = df["lat"].max()

    tree = QuadTree(
        bottomLeft=Point(min_long, min_lat, None),
        topRight=Point(max_long, max_lat, None),
    )
    for idx, row in df.iterrows():
        tree.add_point(Point(row["lng"], row["lat"], row["id"]))

    offset = 0.5
    pairs: List[Tuple[str, str]] = []
    centers = [Point(row["lng"], row["lat"], row["id"]) for idx, row in df.iterrows()]

    def collect_pairs(center: Point, nearby: Point):
        if nearby.value == center.value:
            return
        pairs.append((center.value, nearby.value))

    for_each_nearby_rect(tree, centers, offset, collect_pairs)

    # deduplicate
    pairs = set((a, b) if a <= b else (b, a) for a, b in pairs)
    # group by first stops name
    pair_map: Mapping[str, List[str]] = {}
    for pair in pairs:
        entry = pair_map.get(pair[0])
        if entry == None:
            pair_map[pair[0]] = [pair[1]]
        else:
            entry.append(pair[1])

    edges: List[Edge] = []
    requests: List[Request] = []
    client = openrouteservice.Client(base_url="http://localhost:8082/ors")
    for source, all_destinations in pair_map.items():
        for destinations in chunk(all_destinations, 99):
            batch_size = len(destinations)
            coords = [(stop_map[source].lng, stop_map[source].lat)]
            for dest in destinations:
                coords.append((stop_map[dest].lng, stop_map[dest].lat))

            source_idx = [0]
            dest_idx = [i + 1 for i in range(batch_size)]
            requests.append(
                Request(
                    client=client,
                    coords=coords,
                    sources_idx=source_idx,
                    destinations_idx=dest_idx,
                    destinations=destinations,
                    source=source,
                )
            )

    with ThreadPoolExecutor() as pool:
        futures = {pool.submit(request_edge, r): r for r in requests}
        for future in tqdm(as_completed(futures), total=len(futures)):
            local_edges = future.result()
            edges.extend(local_edges)

    edges = [e for e in edges if e.distance != None and e.duration != None]

    with open("walk.json", "w", encoding="utf-8") as f:
        json.dump([e.__dict__ for e in edges], f, ensure_ascii=False)
